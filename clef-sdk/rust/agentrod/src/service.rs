use std::{collections::BTreeMap, fmt};

use agentro_contracts::{CapabilityName, RequestId, Sha256Digest};
use clef_agent::{
    AgentBackend, AgentCapability, AgentContractError, AgentError, AgentErrorCode, AgentEvent,
    AgentEventBody, AgentEventValidator, AgentInput, AgentSession, CancelReason, CapabilityRequest,
    MAX_AGENT_EVENT_BYTES, OpenSessionRequest, ProbeLevel, ProbeRequest, ProtocolVersion,
    ProviderName, WorkspaceId,
};
use clef_core::{
    Attempt, AttemptCommand, AttemptNumber, CompileContext, CompileError, ExecutionPlan,
    PublishDecision, PublishGate, PublishRequest, ReadyScheduler, RunId, SchedulerError, TaskId,
    TaskOutcome, TransitionError, WorkflowScheduleState, compile_workflow,
};

use crate::api::{
    CANCEL_REQUESTS_PER_RUN, CancelRunRequest, CompileWorkflowRequest, CompileWorkflowResponse,
    CoordinateError, MAX_AGENTROD_TASKS, RunEvent, RunEventBody, RunFailure, RunSnapshot, RunState,
    ServiceLimits, StartRunRequest, WatchRunPage, WatchRunRequest, derive_session_coordinates,
    snapshots,
};

struct CompiledEntry {
    plan: ExecutionPlan,
}

struct OwnedSession {
    session: Box<dyn AgentSession>,
    validator: AgentEventValidator,
}

struct RunRecord {
    run_id: RunId,
    plan: ExecutionPlan,
    workspace_id: WorkspaceId,
    provider: ProviderName,
    scheduler: ReadyScheduler,
    attempts: BTreeMap<TaskId, Attempt>,
    sessions: BTreeMap<TaskId, OwnedSession>,
    state: RunState,
    events: Vec<RunEvent>,
    event_capacity: usize,
    terminal_emitted: bool,
}

impl RunRecord {
    fn append(&mut self, body: RunEventBody) -> Result<(), AgentrodError> {
        if self.events.len() >= self.event_capacity {
            return Err(AgentrodError::EventCapacityExhausted);
        }
        let sequence = u64::try_from(self.events.len())
            .map_err(|_| AgentrodError::EventCapacityExhausted)?
            .saturating_add(1);
        self.events
            .push(RunEvent::new(self.run_id.clone(), sequence, body));
        Ok(())
    }

    fn snapshot(&self) -> RunSnapshot {
        RunSnapshot::new(
            self.run_id.clone(),
            self.plan.digest(),
            self.state,
            self.events
                .last()
                .map(RunEvent::sequence)
                .unwrap_or_default(),
            snapshots(
                self.scheduler.states(),
                &self.attempts,
                self.plan
                    .tasks()
                    .iter()
                    .map(|task| task.spec().id().clone()),
            ),
        )
    }
}

/// Single-owner bounded application service behind the future gRPC handlers.
///
/// `Agentrod` creates no runtime and spawns no task. The daemon/application
/// owner calls [`advance_run`](Self::advance_run) from its supervised loop and
/// owns shutdown/cancellation deadlines around backend operations.
pub struct Agentrod<B, G> {
    backend: B,
    publish_gate: G,
    limits: ServiceLimits,
    plans: BTreeMap<Sha256Digest, CompiledEntry>,
    compile_requests: BTreeMap<RequestId, (CompileWorkflowRequest, CompileWorkflowResponse)>,
    runs: BTreeMap<RunId, RunRecord>,
    start_requests: BTreeMap<RequestId, StartRunRequest>,
    cancel_request_owners: BTreeMap<RequestId, RunId>,
}

impl<B, G> Agentrod<B, G>
where
    B: AgentBackend,
    G: PublishGate,
{
    /// Creates a service with explicit backend, publish gate, and hard limits.
    #[must_use]
    pub fn new(backend: B, publish_gate: G, limits: ServiceLimits) -> Self {
        Self {
            backend,
            publish_gate,
            limits,
            plans: BTreeMap::new(),
            compile_requests: BTreeMap::new(),
            runs: BTreeMap::new(),
            start_requests: BTreeMap::new(),
            cancel_request_owners: BTreeMap::new(),
        }
    }

    /// Probes normalized capabilities, compiles deterministically, and retains
    /// one bounded immutable plan.
    ///
    /// Replaying the same request ID and payload returns the first response;
    /// reusing the ID for another payload fails closed.
    ///
    /// # Errors
    ///
    /// Returns a typed idempotency, probe, compile, or capacity error.
    pub fn compile_workflow(
        &mut self,
        request: CompileWorkflowRequest,
    ) -> Result<CompileWorkflowResponse, AgentrodError> {
        if let Some((previous, response)) = self.compile_requests.get(&request.request_id()) {
            return if previous == &request {
                Ok(response.clone())
            } else {
                Err(AgentrodError::IdempotencyKeyReused)
            };
        }
        if self.compile_requests.len() >= self.limits.max_plans() {
            return Err(AgentrodError::PlanCapacityExhausted);
        }

        let report = self
            .backend
            .probe(ProbeRequest::new(ProbeLevel::Protocol))?;
        validate_probe(&report)?;
        let plan = compile_workflow(
            request.definition(),
            &CompileContext::new(report.capabilities().clone(), request.profile_revision()),
        )?;
        if !self.plans.contains_key(&plan.digest()) && self.plans.len() >= self.limits.max_plans() {
            return Err(AgentrodError::PlanCapacityExhausted);
        }
        let response = CompileWorkflowResponse::new(
            plan.workflow_id().clone(),
            plan.digest(),
            plan.tasks()
                .iter()
                .map(|task| task.spec().id().clone())
                .collect(),
        );
        self.plans
            .entry(plan.digest())
            .or_insert(CompiledEntry { plan });
        self.compile_requests
            .insert(request.request_id(), (request, response.clone()));
        Ok(response)
    }

    /// Creates a run resource, records `RunStarted`, and opens the first bounded
    /// deterministic ready set.
    ///
    /// The command does not consume the full event stream; callers drive at most
    /// one event per active session with [`advance_run`](Self::advance_run).
    ///
    /// # Errors
    ///
    /// Returns a typed idempotency, plan/run, capability, or capacity error.
    pub fn start_run(&mut self, request: StartRunRequest) -> Result<RunSnapshot, AgentrodError> {
        if let Some(previous) = self.start_requests.get(&request.request_id()) {
            return if previous == &request {
                self.get_run(request.run_id())
            } else {
                Err(AgentrodError::IdempotencyKeyReused)
            };
        }
        if self.runs.contains_key(request.run_id()) {
            return Err(AgentrodError::RunAlreadyExists);
        }
        if self.runs.len() >= self.limits.max_runs() {
            return Err(AgentrodError::RunCapacityExhausted);
        }
        let plan = self
            .plans
            .get(&request.plan_digest())
            .ok_or(AgentrodError::PlanNotFound)?
            .plan
            .clone();
        if plan.tasks().len() > MAX_AGENTROD_TASKS {
            return Err(AgentrodError::RunTaskLimitExceeded);
        }
        let required_events = 2_usize.saturating_add(
            plan.tasks()
                .len()
                .saturating_mul(self.limits.max_agent_events_per_task() as usize + 2),
        );
        if required_events > self.limits.max_events_per_run() {
            return Err(AgentrodError::EventCapacityTooSmall);
        }

        let report = self
            .backend
            .probe(ProbeRequest::new(ProbeLevel::Protocol))?;
        validate_probe(&report)?;
        let mut record = RunRecord {
            run_id: request.run_id().clone(),
            scheduler: ReadyScheduler::new(&plan),
            plan,
            workspace_id: request.workspace_id().clone(),
            provider: report.provider().clone(),
            attempts: BTreeMap::new(),
            sessions: BTreeMap::new(),
            state: RunState::Running,
            events: Vec::with_capacity(required_events),
            event_capacity: self.limits.max_events_per_run(),
            terminal_emitted: false,
        };
        record.append(RunEventBody::RunStarted)?;
        if let Err(error) = admit_tasks(
            &self.backend,
            &mut record,
            self.limits.max_agent_events_per_task(),
        ) {
            let _ = cancel_active(&mut record, CancelReason::Shutdown);
            return Err(error);
        }
        sync_terminal(&mut record)?;
        let snapshot = record.snapshot();
        self.start_requests
            .insert(request.request_id(), request.clone());
        self.runs.insert(request.run_id().clone(), record);
        Ok(snapshot)
    }

    /// Polls at most one event from each active session, validates ordering,
    /// applies attempt/publish transitions, and admits newly ready tasks.
    ///
    /// Terminal runs are returned unchanged, making repeated drive calls safe.
    ///
    /// # Errors
    ///
    /// Returns a typed lookup, backend, transition, protocol, or capacity error.
    pub fn advance_run(&mut self, run_id: &RunId) -> Result<RunSnapshot, AgentrodError> {
        let backend = &self.backend;
        let publish_gate = &self.publish_gate;
        let max_agent_events = self.limits.max_agent_events_per_task();
        let record = self
            .runs
            .get_mut(run_id)
            .ok_or(AgentrodError::RunNotFound)?;
        if record.state.is_terminal() {
            return Ok(record.snapshot());
        }

        let active: Vec<TaskId> = record
            .plan
            .tasks()
            .iter()
            .map(|task| task.spec().id().clone())
            .filter(|task_id| record.sessions.contains_key(task_id))
            .collect();
        for task_id in active {
            let polled = match record.sessions.get_mut(&task_id) {
                Some(owned) => owned.session.poll_event(),
                None => continue,
            };
            match polled {
                Ok(Some(event)) => {
                    let validation = record
                        .sessions
                        .get_mut(&task_id)
                        .map(|owned| owned.validator.accept(&event));
                    match validation {
                        Some(Ok(())) => process_valid_event(record, publish_gate, &task_id, event)?,
                        Some(Err(error)) => {
                            fail_task(record, &task_id, RunFailure::Protocol(error))?;
                        }
                        None => {}
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    fail_task(record, &task_id, RunFailure::Agent(error.code()))?;
                }
            }
            if record.plan.policy().is_fail_fast()
                && record.scheduler.workflow_state() == WorkflowScheduleState::Running
                && record
                    .scheduler
                    .states()
                    .values()
                    .any(|state| *state == clef_core::TaskScheduleState::Failed)
            {
                cancel_active(record, CancelReason::Shutdown)?;
            }
        }

        if record.state == RunState::Running {
            admit_tasks(backend, record, max_agent_events)?;
            sync_terminal(record)?;
        }
        Ok(record.snapshot())
    }

    /// Returns one run snapshot without advancing backend/session state.
    ///
    /// # Errors
    ///
    /// Returns [`AgentrodError::RunNotFound`] for an unknown run.
    pub fn get_run(&self, run_id: &RunId) -> Result<RunSnapshot, AgentrodError> {
        self.runs
            .get(run_id)
            .map(RunRecord::snapshot)
            .ok_or(AgentrodError::RunNotFound)
    }

    /// Returns a bounded ordered page strictly after the caller's sequence.
    ///
    /// # Errors
    ///
    /// Returns a lookup, cursor, or service watch-limit error.
    pub fn watch_run(&self, request: &WatchRunRequest) -> Result<WatchRunPage, AgentrodError> {
        let record = self
            .runs
            .get(request.run_id())
            .ok_or(AgentrodError::RunNotFound)?;
        let after = usize::try_from(request.after_sequence())
            .map_err(|_| AgentrodError::InvalidWatchCursor)?;
        if after > record.events.len() {
            return Err(AgentrodError::InvalidWatchCursor);
        }
        let limit = request.limit().min(self.limits.max_watch_events());
        let end = after.saturating_add(limit).min(record.events.len());
        let events = record.events[after..end].to_vec();
        Ok(WatchRunPage::new(
            events,
            record
                .events
                .last()
                .map(RunEvent::sequence)
                .unwrap_or_default(),
            end < record.events.len(),
        ))
    }

    /// Idempotently cancels all active sessions, explicitly closes them, and
    /// records one cancelled terminal event.
    ///
    /// # Errors
    ///
    /// Returns a typed request reuse, lookup, backend cleanup, transition, or
    /// capacity error. Cleanup continues across individual session failures.
    pub fn cancel_run(&mut self, request: CancelRunRequest) -> Result<RunSnapshot, AgentrodError> {
        if let Some(owner) = self.cancel_request_owners.get(&request.request_id()) {
            return if owner == request.run_id() {
                self.get_run(request.run_id())
            } else {
                Err(AgentrodError::IdempotencyKeyReused)
            };
        }
        let run_request_count = self
            .cancel_request_owners
            .values()
            .filter(|owner| *owner == request.run_id())
            .count();
        if run_request_count >= CANCEL_REQUESTS_PER_RUN
            || self.cancel_request_owners.len() >= self.limits.max_cancel_requests()
        {
            return Err(AgentrodError::CancelRequestCapacityExhausted);
        }
        let record = self
            .runs
            .get_mut(request.run_id())
            .ok_or(AgentrodError::RunNotFound)?;
        if !record.state.is_terminal() {
            cancel_active(record, CancelReason::User)?;
            record.scheduler.cancel_all();
            record.state = RunState::Cancelled;
            if !record.terminal_emitted {
                record.append(RunEventBody::RunCancelled)?;
                record.terminal_emitted = true;
            }
        }
        let snapshot = record.snapshot();
        self.cancel_request_owners
            .insert(request.request_id(), request.run_id().clone());
        Ok(snapshot)
    }
}

fn validate_probe(report: &clef_agent::ProbeReport) -> Result<(), AgentrodError> {
    if report.protocol().major() != ProtocolVersion::V1.major() {
        return Err(AgentrodError::Agent(AgentErrorCode::ProtocolViolation));
    }
    if !report.is_authenticated() {
        return Err(AgentrodError::Agent(AgentErrorCode::AuthenticationMissing));
    }
    Ok(())
}

fn task_capabilities(task: &clef_core::TaskSpec) -> Result<CapabilityRequest, AgentrodError> {
    let mut required = task.required_capabilities().to_vec();
    if task.effort().is_some() {
        let effort = CapabilityName::parse(AgentCapability::ReasoningEffort.as_str())?;
        if !required.contains(&effort) {
            required.push(effort);
        }
    }
    CapabilityRequest::new(required, task.preferred_capabilities().to_vec()).map_err(Into::into)
}

fn admit_tasks<B: AgentBackend>(
    backend: &B,
    record: &mut RunRecord,
    max_agent_events: u32,
) -> Result<(), AgentrodError> {
    let admitted = record.scheduler.admit_ready();
    for task_id in admitted {
        let Some(compiled) = record.plan.task(&task_id) else {
            fail_task(
                record,
                &task_id,
                RunFailure::Agent(AgentErrorCode::Internal),
            )?;
            continue;
        };
        let task = compiled.spec().clone();
        let attempt_number = AttemptNumber::new(1)?;
        let mut attempt = Attempt::new(task_id.clone(), attempt_number);
        attempt.apply(AttemptCommand::Begin {
            approval_required: false,
        })?;
        record.attempts.insert(task_id.clone(), attempt);
        record.append(RunEventBody::TaskStarted {
            task_id: task_id.clone(),
            attempt: attempt_number.value(),
        })?;

        let coordinates = match derive_session_coordinates(&record.run_id, &task_id) {
            Ok(value) => value,
            Err(_) => {
                fail_task(
                    record,
                    &task_id,
                    RunFailure::Agent(AgentErrorCode::Internal),
                )?;
                continue;
            }
        };
        let capabilities = match task_capabilities(&task) {
            Ok(value) => value,
            Err(_) => {
                fail_task(
                    record,
                    &task_id,
                    RunFailure::Agent(AgentErrorCode::Internal),
                )?;
                continue;
            }
        };
        let open_request = match OpenSessionRequest::new(
            coordinates.session_id().clone(),
            record.workspace_id.clone(),
            capabilities,
            task.effort(),
            max_agent_events,
        ) {
            Ok(value) => value,
            Err(_) => {
                fail_task(
                    record,
                    &task_id,
                    RunFailure::Agent(AgentErrorCode::Internal),
                )?;
                continue;
            }
        };
        let mut session = match backend.open_session(open_request) {
            Ok(value) => value,
            Err(error) => {
                fail_task(record, &task_id, RunFailure::Agent(error.code()))?;
                continue;
            }
        };
        let input = match AgentInput::new(coordinates.turn_id().clone(), task.instruction()) {
            Ok(value) => value,
            Err(_) => {
                let _ = session.cancel(CancelReason::Shutdown);
                let _ = session.close();
                fail_task(
                    record,
                    &task_id,
                    RunFailure::Agent(AgentErrorCode::Internal),
                )?;
                continue;
            }
        };
        if let Err(error) = session.send(input) {
            let _ = session.cancel(CancelReason::Shutdown);
            let _ = session.close();
            fail_task(record, &task_id, RunFailure::Agent(error.code()))?;
            continue;
        }
        let validator = AgentEventValidator::new(
            coordinates.session_id().clone(),
            coordinates.turn_id().clone(),
            record.provider.clone(),
            MAX_AGENT_EVENT_BYTES,
            max_agent_events,
        )?;
        record
            .sessions
            .insert(task_id, OwnedSession { session, validator });
    }
    Ok(())
}

fn process_valid_event<G: PublishGate>(
    record: &mut RunRecord,
    publish_gate: &G,
    task_id: &TaskId,
    event: AgentEvent,
) -> Result<(), AgentrodError> {
    let body = event.body().clone();
    record.append(RunEventBody::Agent {
        task_id: task_id.clone(),
        event,
    })?;
    match body {
        AgentEventBody::TurnCompleted { artifacts } => {
            if let Err(error) = close_terminal_session(record, task_id) {
                fail_task(record, task_id, RunFailure::Agent(error.code()))?;
                return Ok(());
            }
            let produced = artifacts
                .into_iter()
                .map(|artifact| (artifact.name().clone(), artifact))
                .collect();
            let Some(attempt) = record.attempts.get_mut(task_id) else {
                return Err(AgentrodError::InternalInvariant);
            };
            attempt.apply(AttemptCommand::BeginVerification)?;
            attempt.apply(AttemptCommand::BeginPublishing)?;
            let Some(task) = record.plan.task(task_id) else {
                return Err(AgentrodError::InternalInvariant);
            };
            match publish_gate.evaluate(PublishRequest::new(
                task.spec(),
                attempt.number(),
                &produced,
                true,
            )) {
                PublishDecision::Approved => {
                    attempt.apply(AttemptCommand::Complete)?;
                    record.scheduler.finish(task_id, TaskOutcome::Succeeded)?;
                    record.append(RunEventBody::TaskPublished {
                        task_id: task_id.clone(),
                    })?;
                }
                PublishDecision::Rejected(rejection) => {
                    attempt.apply(AttemptCommand::Fail)?;
                    record.scheduler.finish(task_id, TaskOutcome::Failed)?;
                    record.append(RunEventBody::TaskFailed {
                        task_id: task_id.clone(),
                        failure: RunFailure::Publish(rejection),
                    })?;
                }
            }
        }
        AgentEventBody::TurnFailed { code } => {
            fail_task(record, task_id, RunFailure::Agent(code))?;
        }
        _ => {}
    }
    Ok(())
}

fn close_terminal_session(record: &mut RunRecord, task_id: &TaskId) -> Result<(), AgentError> {
    if let Some(mut owned) = record.sessions.remove(task_id) {
        let post_terminal = owned.session.poll_event();
        let close = owned.session.close();
        match post_terminal {
            Ok(Some(_)) => {
                return Err(AgentError::new(AgentErrorCode::ProtocolViolation));
            }
            Err(error) => return Err(error),
            Ok(None) => {}
        }
        close?;
    }
    Ok(())
}

fn fail_task(
    record: &mut RunRecord,
    task_id: &TaskId,
    failure: RunFailure,
) -> Result<(), AgentrodError> {
    if let Some(mut owned) = record.sessions.remove(task_id) {
        let _ = owned.session.cancel(CancelReason::Shutdown);
        let _ = owned.session.close();
    }
    let Some(attempt) = record.attempts.get_mut(task_id) else {
        return Err(AgentrodError::InternalInvariant);
    };
    if !attempt.state().is_terminal() {
        attempt.apply(AttemptCommand::Fail)?;
    }
    if record.scheduler.state(task_id) == Some(clef_core::TaskScheduleState::Running) {
        record.scheduler.finish(task_id, TaskOutcome::Failed)?;
    }
    record.append(RunEventBody::TaskFailed {
        task_id: task_id.clone(),
        failure,
    })?;
    Ok(())
}

fn cancel_active(record: &mut RunRecord, reason: CancelReason) -> Result<(), AgentrodError> {
    let active: Vec<TaskId> = record
        .plan
        .tasks()
        .iter()
        .map(|task| task.spec().id().clone())
        .filter(|task_id| record.sessions.contains_key(task_id))
        .collect();
    for task_id in active {
        let mut cleanup_error = None;
        if let Some(mut owned) = record.sessions.remove(&task_id) {
            if let Err(error) = owned.session.cancel(reason) {
                cleanup_error.get_or_insert(error.code());
            }
            if let Err(error) = owned.session.close() {
                cleanup_error.get_or_insert(error.code());
            }
        }
        if let Some(attempt) = record.attempts.get_mut(&task_id)
            && !attempt.state().is_terminal()
        {
            attempt.apply(AttemptCommand::Cancel)?;
        }
        if record.scheduler.state(&task_id) == Some(clef_core::TaskScheduleState::Running) {
            record.scheduler.finish(&task_id, TaskOutcome::Cancelled)?;
        }
        record.append(RunEventBody::TaskCancelled {
            task_id,
            cleanup_error,
        })?;
    }
    Ok(())
}

fn sync_terminal(record: &mut RunRecord) -> Result<(), AgentrodError> {
    if record.terminal_emitted || !record.sessions.is_empty() {
        return Ok(());
    }
    match record.scheduler.workflow_state() {
        WorkflowScheduleState::Succeeded => {
            record.state = RunState::Succeeded;
            record.append(RunEventBody::RunSucceeded)?;
            record.terminal_emitted = true;
        }
        WorkflowScheduleState::Failed => {
            record.state = RunState::Failed;
            record.append(RunEventBody::RunFailed)?;
            record.terminal_emitted = true;
        }
        WorkflowScheduleState::Cancelled => {
            record.state = RunState::Cancelled;
            record.append(RunEventBody::RunCancelled)?;
            record.terminal_emitted = true;
        }
        WorkflowScheduleState::Pending | WorkflowScheduleState::Running => {}
    }
    Ok(())
}

/// Stable application failure without provider-private payloads.
#[derive(Debug)]
pub enum AgentrodError {
    /// An idempotency key was reused with another payload/scope.
    IdempotencyKeyReused,
    /// Compiled plan does not exist in this service instance.
    PlanNotFound,
    /// Run does not exist in this service instance.
    RunNotFound,
    /// Caller attempted to create an existing run ID.
    RunAlreadyExists,
    /// Retained plan capacity is full.
    PlanCapacityExhausted,
    /// Retained run capacity is full.
    RunCapacityExhausted,
    /// Retained cancellation idempotency capacity is full.
    CancelRequestCapacityExhausted,
    /// Plan exceeds the first vertical slice's task bound.
    RunTaskLimitExceeded,
    /// Configured event capacity cannot hold worst-case non-droppable events.
    EventCapacityTooSmall,
    /// A run consumed its preallocated non-droppable event capacity.
    EventCapacityExhausted,
    /// Watch cursor is beyond the run's current event sequence.
    InvalidWatchCursor,
    /// Normalized backend failure.
    Agent(AgentErrorCode),
    /// Static workflow compilation failed.
    Compile(CompileError),
    /// Attempt transition failed.
    Transition(TransitionError),
    /// Ready scheduler transition failed.
    Scheduler(SchedulerError),
    /// Agent contract construction failed.
    AgentContract(AgentContractError),
    /// Built-in common capability name failed validation.
    Capability(agentro_contracts::CapabilityError),
    /// Deterministic session/turn ID derivation failed.
    Coordinate(CoordinateError),
    /// An application invariant failed after static validation.
    InternalInvariant,
}

impl fmt::Display for AgentrodError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdempotencyKeyReused => formatter.write_str("idempotency key was reused"),
            Self::PlanNotFound => formatter.write_str("compiled plan was not found"),
            Self::RunNotFound => formatter.write_str("run was not found"),
            Self::RunAlreadyExists => formatter.write_str("run already exists"),
            Self::PlanCapacityExhausted => formatter.write_str("plan capacity is exhausted"),
            Self::RunCapacityExhausted => formatter.write_str("run capacity is exhausted"),
            Self::CancelRequestCapacityExhausted => {
                formatter.write_str("cancel request capacity is exhausted")
            }
            Self::RunTaskLimitExceeded => formatter.write_str("run task limit is exceeded"),
            Self::EventCapacityTooSmall => formatter.write_str("event capacity is too small"),
            Self::EventCapacityExhausted => formatter.write_str("event capacity is exhausted"),
            Self::InvalidWatchCursor => formatter.write_str("watch cursor is invalid"),
            Self::Agent(code) => write!(formatter, "normalized backend failed: {code:?}"),
            Self::Compile(error) => write!(formatter, "workflow compile failed: {error}"),
            Self::Transition(error) => write!(formatter, "attempt transition failed: {error}"),
            Self::Scheduler(error) => write!(formatter, "scheduler transition failed: {error}"),
            Self::AgentContract(error) => write!(formatter, "agent contract failed: {error}"),
            Self::Capability(error) => write!(formatter, "capability failed: {error}"),
            Self::Coordinate(error) => write!(formatter, "coordinate failed: {error}"),
            Self::InternalInvariant => formatter.write_str("agentrod invariant failed"),
        }
    }
}

impl std::error::Error for AgentrodError {}

impl From<AgentError> for AgentrodError {
    fn from(error: AgentError) -> Self {
        Self::Agent(error.code())
    }
}

impl From<CompileError> for AgentrodError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

impl From<TransitionError> for AgentrodError {
    fn from(error: TransitionError) -> Self {
        Self::Transition(error)
    }
}

impl From<SchedulerError> for AgentrodError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

impl From<AgentContractError> for AgentrodError {
    fn from(error: AgentContractError) -> Self {
        Self::AgentContract(error)
    }
}

impl From<agentro_contracts::CapabilityError> for AgentrodError {
    fn from(error: agentro_contracts::CapabilityError) -> Self {
        Self::Capability(error)
    }
}

impl From<CoordinateError> for AgentrodError {
    fn from(error: CoordinateError) -> Self {
        Self::Coordinate(error)
    }
}
