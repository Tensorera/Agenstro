use std::{collections::BTreeMap, fmt};

use agentro_contracts::{CanonicalHasher, DigestError, RequestId, Sha256Digest};
use clef_agent::{
    AgentContractError, AgentErrorCode, AgentEvent, AgentProtocolError, AgentSessionId,
    AgentTurnId, WorkspaceId,
};
use clef_core::{AttemptState, PublishRejection, RunId, TaskId, TaskScheduleState, WorkflowSpec};

/// Maximum tasks admitted by the first in-memory application vertical slice.
pub const MAX_AGENTROD_TASKS: usize = 64;
pub(crate) const CANCEL_REQUESTS_PER_RUN: usize = 4;

/// Explicit hard limits for all service-owned in-memory collections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceLimits {
    max_plans: usize,
    max_runs: usize,
    max_events_per_run: usize,
    max_watch_events: usize,
    max_agent_events_per_task: u32,
}

impl ServiceLimits {
    /// Creates validated application limits.
    ///
    /// # Errors
    ///
    /// Rejects zero values, more than 256 plans/runs/watch items, more than
    /// 8,192 retained events, or more than 16 backend events per task.
    pub const fn new(
        max_plans: usize,
        max_runs: usize,
        max_events_per_run: usize,
        max_watch_events: usize,
        max_agent_events_per_task: u32,
    ) -> Result<Self, ApiValueError> {
        if max_plans == 0
            || max_plans > 256
            || max_runs == 0
            || max_runs > 256
            || max_events_per_run == 0
            || max_events_per_run > 8_192
            || max_watch_events == 0
            || max_watch_events > 256
            || max_agent_events_per_task == 0
            || max_agent_events_per_task > 16
        {
            return Err(ApiValueError::InvalidLimit);
        }
        Ok(Self {
            max_plans,
            max_runs,
            max_events_per_run,
            max_watch_events,
            max_agent_events_per_task,
        })
    }

    /// Returns the maximum compiled plans.
    #[must_use]
    pub const fn max_plans(self) -> usize {
        self.max_plans
    }

    /// Returns the maximum retained runs.
    #[must_use]
    pub const fn max_runs(self) -> usize {
        self.max_runs
    }

    /// Returns the maximum ordered events retained for one run.
    #[must_use]
    pub const fn max_events_per_run(self) -> usize {
        self.max_events_per_run
    }

    /// Returns the maximum events in one watch response.
    #[must_use]
    pub const fn max_watch_events(self) -> usize {
        self.max_watch_events
    }

    /// Returns the hard accepted adapter events per task turn.
    #[must_use]
    pub const fn max_agent_events_per_task(self) -> u32 {
        self.max_agent_events_per_task
    }

    /// Returns the hard number of retained cancellation idempotency records.
    ///
    /// Four retries are reserved per possible run so one run cannot consume
    /// another live run's cancellation capacity.
    #[must_use]
    pub const fn max_cancel_requests(self) -> usize {
        self.max_runs * CANCEL_REQUESTS_PER_RUN
    }
}

impl Default for ServiceLimits {
    fn default() -> Self {
        Self {
            max_plans: 64,
            max_runs: 64,
            max_events_per_run: 2_048,
            max_watch_events: 256,
            max_agent_events_per_task: 16,
        }
    }
}

/// A bounded public API value is invalid before application execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiValueError {
    /// A service resource limit is outside its supported range.
    InvalidLimit,
    /// A watch page size is zero or larger than 256.
    InvalidWatchLimit,
}

impl fmt::Display for ApiValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLimit => "agentrod service limit is invalid",
            Self::InvalidWatchLimit => "agentrod watch limit is invalid",
        })
    }
}

impl std::error::Error for ApiValueError {}

/// Idempotent request to compile and retain one workflow plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileWorkflowRequest {
    request_id: RequestId,
    definition: WorkflowSpec,
    profile_revision: Sha256Digest,
}

impl CompileWorkflowRequest {
    /// Creates a compile command from immutable values.
    #[must_use]
    pub const fn new(
        request_id: RequestId,
        definition: WorkflowSpec,
        profile_revision: Sha256Digest,
    ) -> Self {
        Self {
            request_id,
            definition,
            profile_revision,
        }
    }

    /// Returns the idempotency key.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the immutable workflow definition.
    #[must_use]
    pub const fn definition(&self) -> &WorkflowSpec {
        &self.definition
    }

    /// Returns the profile revision included in deterministic compilation.
    #[must_use]
    pub const fn profile_revision(&self) -> Sha256Digest {
        self.profile_revision
    }
}

/// Stable result of workflow compilation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileWorkflowResponse {
    workflow_id: clef_core::WorkflowId,
    plan_digest: Sha256Digest,
    topological_order: Vec<TaskId>,
}

impl CompileWorkflowResponse {
    pub(crate) fn new(
        workflow_id: clef_core::WorkflowId,
        plan_digest: Sha256Digest,
        topological_order: Vec<TaskId>,
    ) -> Self {
        Self {
            workflow_id,
            plan_digest,
            topological_order,
        }
    }

    /// Returns the compiled workflow identity.
    #[must_use]
    pub const fn workflow_id(&self) -> &clef_core::WorkflowId {
        &self.workflow_id
    }

    /// Returns the canonical plan digest.
    #[must_use]
    pub const fn plan_digest(&self) -> Sha256Digest {
        self.plan_digest
    }

    /// Returns deterministic task order.
    #[must_use]
    pub fn topological_order(&self) -> &[TaskId] {
        &self.topological_order
    }
}

/// Idempotent command to create and start one workflow run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartRunRequest {
    request_id: RequestId,
    run_id: RunId,
    plan_digest: Sha256Digest,
    workspace_id: WorkspaceId,
}

impl StartRunRequest {
    /// Creates a start command from validated values.
    #[must_use]
    pub const fn new(
        request_id: RequestId,
        run_id: RunId,
        plan_digest: Sha256Digest,
        workspace_id: WorkspaceId,
    ) -> Self {
        Self {
            request_id,
            run_id,
            plan_digest,
            workspace_id,
        }
    }

    /// Returns the idempotency key.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the caller-selected stable run identity.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact retained plan to execute.
    #[must_use]
    pub const fn plan_digest(&self) -> Sha256Digest {
        self.plan_digest
    }

    /// Returns the authorized opaque workspace identity.
    #[must_use]
    pub const fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
}

/// Idempotent cancellation command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelRunRequest {
    request_id: RequestId,
    run_id: RunId,
}

impl CancelRunRequest {
    /// Creates a cancellation command.
    #[must_use]
    pub const fn new(request_id: RequestId, run_id: RunId) -> Self {
        Self { request_id, run_id }
    }

    /// Returns the idempotency key.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the target run.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

/// Bounded watch query for events strictly after one sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchRunRequest {
    run_id: RunId,
    after_sequence: u64,
    limit: usize,
}

impl WatchRunRequest {
    /// Creates a bounded watch request.
    ///
    /// # Errors
    ///
    /// Rejects `limit == 0` or `limit > 256`.
    pub fn new(run_id: RunId, after_sequence: u64, limit: usize) -> Result<Self, ApiValueError> {
        if limit == 0 || limit > 256 {
            return Err(ApiValueError::InvalidWatchLimit);
        }
        Ok(Self {
            run_id,
            after_sequence,
            limit,
        })
    }

    /// Returns the target run.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the last sequence already applied by the caller.
    #[must_use]
    pub const fn after_sequence(&self) -> u64 {
        self.after_sequence
    }

    /// Returns the requested maximum page items.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }
}

/// Aggregate externally visible run state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RunState {
    /// Sessions are active or tasks remain schedulable.
    Running,
    /// Every task published successfully.
    Succeeded,
    /// A task, protocol, or publish gate failed.
    Failed,
    /// Explicit cancellation completed.
    Cancelled,
}

impl RunState {
    /// Returns whether no further advancement is allowed.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Point-in-time task state included in a run snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSnapshot {
    task_id: TaskId,
    schedule_state: TaskScheduleState,
    attempt_state: Option<AttemptState>,
}

impl TaskSnapshot {
    pub(crate) const fn new(
        task_id: TaskId,
        schedule_state: TaskScheduleState,
        attempt_state: Option<AttemptState>,
    ) -> Self {
        Self {
            task_id,
            schedule_state,
            attempt_state,
        }
    }

    /// Returns the task ID.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns scheduler state.
    #[must_use]
    pub const fn schedule_state(&self) -> TaskScheduleState {
        self.schedule_state
    }

    /// Returns the current/terminal attempt state once admitted.
    #[must_use]
    pub const fn attempt_state(&self) -> Option<AttemptState> {
        self.attempt_state
    }
}

/// Point-in-time run resource returned by start/get/cancel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSnapshot {
    run_id: RunId,
    plan_digest: Sha256Digest,
    state: RunState,
    last_sequence: u64,
    tasks: Vec<TaskSnapshot>,
}

impl RunSnapshot {
    pub(crate) fn new(
        run_id: RunId,
        plan_digest: Sha256Digest,
        state: RunState,
        last_sequence: u64,
        tasks: Vec<TaskSnapshot>,
    ) -> Self {
        Self {
            run_id,
            plan_digest,
            state,
            last_sequence,
            tasks,
        }
    }

    /// Returns the stable run ID.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the immutable execution-plan digest.
    #[must_use]
    pub const fn plan_digest(&self) -> Sha256Digest {
        self.plan_digest
    }

    /// Returns aggregate run state.
    #[must_use]
    pub const fn state(&self) -> RunState {
        self.state
    }

    /// Returns the last durable-order sequence in this in-memory slice.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Returns task snapshots in deterministic topological order.
    #[must_use]
    pub fn tasks(&self) -> &[TaskSnapshot] {
        &self.tasks
    }
}

/// Stable reason attached to task/run failure events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunFailure {
    /// Normalized backend failure.
    Agent(AgentErrorCode),
    /// Normalized event stream violated its contract.
    Protocol(AgentProtocolError),
    /// Artifact publication was rejected.
    Publish(PublishRejection),
}

/// One application event body. Provider-private payloads are impossible here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunEventBody {
    /// Run resource was accepted before any task event.
    RunStarted,
    /// A task attempt obtained a bounded scheduling permit.
    TaskStarted {
        /// Admitted task.
        task_id: TaskId,
        /// One-based attempt number.
        attempt: u32,
    },
    /// Validated normalized adapter event.
    Agent {
        /// Owning task.
        task_id: TaskId,
        /// Provider-neutral event.
        event: AgentEvent,
    },
    /// Publish gate approved and task success committed.
    TaskPublished {
        /// Successful task.
        task_id: TaskId,
    },
    /// Task failed before publication.
    TaskFailed {
        /// Failed task.
        task_id: TaskId,
        /// Stable normalized reason.
        failure: RunFailure,
    },
    /// Active task was cancelled and its session closed.
    TaskCancelled {
        /// Cancelled task.
        task_id: TaskId,
        /// Adapter cleanup error, if bounded cancellation/close degraded.
        cleanup_error: Option<AgentErrorCode>,
    },
    /// Unique successful run terminal.
    RunSucceeded,
    /// Unique failed run terminal.
    RunFailed,
    /// Unique cancelled run terminal.
    RunCancelled,
}

/// One monotonically sequenced application event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEvent {
    run_id: RunId,
    sequence: u64,
    body: RunEventBody,
}

impl RunEvent {
    pub(crate) const fn new(run_id: RunId, sequence: u64, body: RunEventBody) -> Self {
        Self {
            run_id,
            sequence,
            body,
        }
    }

    /// Returns the owning run.
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the one-based sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the typed event body.
    #[must_use]
    pub const fn body(&self) -> &RunEventBody {
        &self.body
    }
}

/// Bounded ordered watch response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchRunPage {
    events: Vec<RunEvent>,
    last_sequence: u64,
    has_more: bool,
}

impl WatchRunPage {
    pub(crate) const fn new(events: Vec<RunEvent>, last_sequence: u64, has_more: bool) -> Self {
        Self {
            events,
            last_sequence,
            has_more,
        }
    }

    /// Returns events in strict sequence order.
    #[must_use]
    pub fn events(&self) -> &[RunEvent] {
        &self.events
    }

    /// Returns the resource's current final sequence.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Returns whether another bounded page currently exists.
    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Deterministic core-owned IDs for one run/task session and turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionCoordinates {
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
}

impl SessionCoordinates {
    /// Returns the core-owned session ID.
    #[must_use]
    pub const fn session_id(&self) -> &AgentSessionId {
        &self.session_id
    }

    /// Returns the core-owned turn ID.
    #[must_use]
    pub const fn turn_id(&self) -> &AgentTurnId {
        &self.turn_id
    }
}

/// Derives non-secret deterministic session/turn correlation IDs.
///
/// # Errors
///
/// Returns a canonical digest or bounded agent identifier error only if an
/// internal domain constant violates its contract.
pub fn derive_session_coordinates(
    run_id: &RunId,
    task_id: &TaskId,
) -> Result<SessionCoordinates, CoordinateError> {
    let session_id = derive_coordinate("agentrod.session-id-v1", "session", run_id, task_id)?;
    let turn_id = derive_coordinate("agentrod.turn-id-v1", "turn", run_id, task_id)?;
    Ok(SessionCoordinates {
        session_id: AgentSessionId::parse(&session_id)?,
        turn_id: AgentTurnId::parse(&turn_id)?,
    })
}

fn derive_coordinate(
    domain: &str,
    prefix: &str,
    run_id: &RunId,
    task_id: &TaskId,
) -> Result<String, DigestError> {
    let mut hasher = CanonicalHasher::new(domain)?;
    hasher.write_field("run_id", run_id.as_str().as_bytes())?;
    hasher.write_field("task_id", task_id.as_str().as_bytes())?;
    let hex: String = hasher
        .finish()
        .to_string()
        .chars()
        .skip("sha256:".len())
        .take(32)
        .collect();
    Ok(format!("{prefix}-{hex}"))
}

/// Deterministic ID derivation failure.
#[derive(Debug)]
pub enum CoordinateError {
    /// Canonical digest framing failed.
    Digest(DigestError),
    /// Derived normalized ID violated the agent contract.
    Agent(AgentContractError),
}

impl fmt::Display for CoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Digest(error) => write!(formatter, "coordinate digest failed: {error}"),
            Self::Agent(error) => write!(formatter, "coordinate ID failed: {error}"),
        }
    }
}

impl std::error::Error for CoordinateError {}

impl From<DigestError> for CoordinateError {
    fn from(error: DigestError) -> Self {
        Self::Digest(error)
    }
}

impl From<AgentContractError> for CoordinateError {
    fn from(error: AgentContractError) -> Self {
        Self::Agent(error)
    }
}

pub(crate) fn snapshots(
    states: &BTreeMap<TaskId, TaskScheduleState>,
    attempts: &BTreeMap<TaskId, clef_core::Attempt>,
    order: impl Iterator<Item = TaskId>,
) -> Vec<TaskSnapshot> {
    order
        .filter_map(|task_id| {
            states.get(&task_id).copied().map(|schedule_state| {
                TaskSnapshot::new(
                    task_id.clone(),
                    schedule_state,
                    attempts.get(&task_id).map(clef_core::Attempt::state),
                )
            })
        })
        .collect()
}
