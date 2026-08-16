use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fs, io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use agentro_contracts::{CapabilitySet, RequestId, Sha256Digest, UtcTimestamp};
use agentro_proto::agentro::workflow::v1 as wire;
use agentrod::{
    Agentrod, CompileWorkflowRequest, RunEventBody, RunState, ServiceLimits, StartRunRequest,
    WatchRunRequest,
};
use clef_agent::{
    AgentBackend, AgentCapability, AgentError, AgentErrorCode, AgentEvent, AgentEventBody,
    AgentInput, AgentSession, CancelReason, CancelResult, OpenSessionRequest, ProbeReport,
    ProbeRequest, ProtocolVersion, ProviderName, WorkspaceId,
};
use clef_core::{
    ArtifactKind, ArtifactName, ArtifactSpec, DomainFunctionName, EffectKind, EffectPolicy,
    EffectRule, ProjectPath, RequiredOutputsGate, RunId, SchemaVersion, TaskId, TaskSpec,
    WorkflowId, WorkflowPolicy, WorkflowSpec,
};
use prost::Message;
use segno_core::{
    AgentrodPort, DispatchLookup, DispatchRequest, DispatchStart, LeaseOwnerId, Occurrence,
    OrchestrationRunId, PortError, ScheduleRevision, TaskId as SegnoTaskId, UtcInstant,
};
use serde::Deserialize;
use tactus_core::{
    BeginRequest, CancellationToken, CasCheckpointBackend, CellId, CheckpointConfig, DaemonConfig,
    ExecuteRequest, LeaseOwnerId as TactusLeaseOwnerId, ManualClock, OutputStream, PathPolicy,
    ProjectId, RunEventKind as TactusEventKind, RunState as TactusRunState, ScanBudget,
    StateDurability, TactusDaemon, WorkerCommand, WorkerCompletion, WorkerError, WorkerEventSink,
    WorkerPort, WorkerTerminal,
};
use tempfile::tempdir;

const TACTUS_OUTPUT: &[u8] = b"tactus worker output\n";
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    expected: ExpectedFixture,
    fixture_version: String,
    products: Vec<ProductFixture>,
    protocol: ProtocolFixture,
    release_version: String,
    workflow: WorkflowFixture,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFixture {
    max_events: u32,
    tactus_output: String,
    workspace_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductFixture {
    distribution: String,
    entry_points: Vec<String>,
    python_import: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtocolFixture {
    api_major: u32,
    api_minor: u32,
    workflow_proto: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowFixture {
    id: String,
    outputs: Vec<WorkflowOutputFixture>,
    policy: WorkflowPolicyFixture,
    required_capabilities: Vec<String>,
    schema_version: String,
    tasks: Vec<TaskFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowOutputFixture {
    name: String,
    source: BindingFixture,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingFixture {
    output_name: String,
    source_task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowPolicyFixture {
    fail_fast: bool,
    max_concurrency: u32,
    max_fan_out: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskFixture {
    domain_function: String,
    effects: Vec<EffectFixture>,
    effort: Option<String>,
    id: String,
    inputs: Vec<serde_json::Value>,
    outputs: Vec<ArtifactFixture>,
    preferred_capabilities: Vec<String>,
    prompts: Vec<PromptFixture>,
    required_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectFixture {
    kind: String,
    path_glob: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactFixture {
    description: String,
    kind: String,
    name: String,
    path: Option<String>,
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptFixture {
    content: String,
    name: Option<String>,
    priority: i32,
    role: String,
}

#[derive(Debug, Default)]
struct IntegrationMetrics {
    tactus_runs: AtomicUsize,
    tactus_events: AtomicUsize,
    tactus_output_bytes: AtomicUsize,
    closed_sessions: AtomicUsize,
}

struct TactusBackend {
    report: ProbeReport,
    metrics: Arc<IntegrationMetrics>,
}

impl AgentBackend for TactusBackend {
    fn probe(&self, _request: ProbeRequest) -> Result<ProbeReport, AgentError> {
        Ok(self.report.clone())
    }

    fn open_session(
        &self,
        request: OpenSessionRequest,
    ) -> Result<Box<dyn AgentSession>, AgentError> {
        if !request
            .capabilities()
            .missing_required(self.report.capabilities())
            .is_empty()
        {
            return Err(AgentError::new(AgentErrorCode::CapabilityMissing));
        }
        Ok(Box::new(TactusSession {
            session_id: request.session_id().clone(),
            provider: self.report.provider().clone(),
            max_events: request.max_events(),
            events: VecDeque::new(),
            metrics: Arc::clone(&self.metrics),
            sent: false,
            cancelled: false,
        }))
    }
}

struct TactusSession {
    session_id: clef_agent::AgentSessionId,
    provider: ProviderName,
    max_events: u32,
    events: VecDeque<AgentEvent>,
    metrics: Arc<IntegrationMetrics>,
    sent: bool,
    cancelled: bool,
}

impl AgentSession for TactusSession {
    fn send(&mut self, input: AgentInput) -> Result<(), AgentError> {
        if self.sent || self.cancelled {
            return Err(AgentError::new(AgentErrorCode::ProtocolViolation));
        }
        self.sent = true;
        let execution = execute_tactus(input.instruction())
            .map_err(|_| AgentError::new(AgentErrorCode::Internal))?;
        self.metrics.tactus_runs.fetch_add(1, Ordering::SeqCst);
        self.metrics
            .tactus_events
            .fetch_add(execution.event_count, Ordering::SeqCst);
        self.metrics
            .tactus_output_bytes
            .fetch_add(execution.output_bytes, Ordering::SeqCst);

        let turn_id = input.turn_id().clone();
        let bodies = [
            AgentEventBody::SessionStarted,
            AgentEventBody::ContentDelta {
                text: String::from_utf8_lossy(TACTUS_OUTPUT).into_owned().into(),
            },
            AgentEventBody::TurnCompleted {
                artifacts: vec![clef_core::ProducedArtifact::new(
                    ArtifactName::parse("report")
                        .map_err(|_| AgentError::new(AgentErrorCode::Internal))?,
                    ArtifactKind::Text,
                )],
            },
        ];
        if bodies.len() > self.max_events as usize {
            return Err(AgentError::new(AgentErrorCode::OutputLimit));
        }
        for (index, body) in bodies.into_iter().enumerate() {
            let sequence =
                u64::try_from(index).map_err(|_| AgentError::new(AgentErrorCode::Internal))? + 1;
            self.events.push_back(
                AgentEvent::new(
                    ProtocolVersion::V1,
                    self.session_id.clone(),
                    turn_id.clone(),
                    sequence,
                    UtcTimestamp::new(1_800_000_000 + sequence as i64, 0)
                        .map_err(|_| AgentError::new(AgentErrorCode::Internal))?,
                    self.provider.clone(),
                    body,
                )
                .map_err(|_| AgentError::new(AgentErrorCode::Internal))?,
            );
        }
        Ok(())
    }

    fn poll_event(&mut self) -> Result<Option<AgentEvent>, AgentError> {
        if !self.sent {
            return Err(AgentError::new(AgentErrorCode::ProtocolViolation));
        }
        Ok(self.events.pop_front())
    }

    fn cancel(&mut self, _reason: CancelReason) -> Result<CancelResult, AgentError> {
        self.cancelled = true;
        self.events.clear();
        Ok(CancelResult::Acknowledged)
    }

    fn close(self: Box<Self>) -> Result<(), AgentError> {
        self.metrics.closed_sessions.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FakeWorker;

impl WorkerPort for FakeWorker {
    fn execute(
        &self,
        _command: WorkerCommand,
        cancellation: &CancellationToken,
        sink: &mut dyn WorkerEventSink,
    ) -> Result<WorkerCompletion, WorkerError> {
        if cancellation.is_cancelled() {
            return Err(WorkerError::Cancelled);
        }
        sink.output(3, OutputStream::Stdout, TACTUS_OUTPUT)?;
        WorkerCompletion::new(
            Sha256Digest::from_bytes([4; 32]),
            1,
            WorkerTerminal::Succeeded,
        )
    }
}

struct TactusExecution {
    event_count: usize,
    output_bytes: usize,
}

fn execute_tactus(source: &str) -> TestResult<TactusExecution> {
    let temporary = tempdir()?;
    let workspace = temporary.path().join("workspace");
    let state = temporary.path().join("state");
    fs::create_dir(&workspace)?;
    fs::create_dir(&state)?;
    let checkpoint = CasCheckpointBackend::non_git(
        state.join("cas"),
        CheckpointConfig::new(
            PathPolicy::new(32, 255, 4_096)?,
            ScanBudget::new(1_000, 1_048_576, 1_048_576, 32, Duration::from_secs(5))?,
            1_048_576,
            1_000,
            1_048_576,
            100,
        )?,
    )?;
    let config = DaemonConfig::new(
        1,
        64 * 1_024,
        1_048_576,
        128,
        Duration::from_secs(60),
        StateDurability::LocalWal,
    )?;
    let mut daemon = TactusDaemon::open(
        state.join("tactus.sqlite3"),
        config,
        FakeWorker,
        checkpoint,
        Arc::new(ManualClock::new(1_000)),
    )?;
    let begun = daemon.begin(
        BeginRequest::new(
            RequestId::generate(),
            ProjectId::generate(),
            CellId::generate(),
            source.as_bytes().to_vec(),
            workspace.clone(),
            TactusLeaseOwnerId::generate(),
            Duration::from_secs(60),
        ),
        &CancellationToken::new(),
    )?;
    daemon.execute(ExecuteRequest::new(begun.run_id(), workspace))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = daemon.status(begun.run_id())?;
        if snapshot.run_state().is_terminal() {
            if snapshot.run_state() != TactusRunState::Succeeded {
                return Err(io::Error::other("Tactus fake worker did not succeed").into());
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("Tactus fake worker timed out").into());
        }
        thread::yield_now();
    }
    let page = daemon.watch(begun.run_id(), 0, 32)?;
    if page.events().len() > 32
        || page
            .events()
            .windows(2)
            .any(|pair| pair[1].sequence() != pair[0].sequence() + 1)
        || page.events().last().map(tactus_core::RunEvent::kind) != Some(TactusEventKind::Succeeded)
    {
        return Err(io::Error::other("Tactus returned an invalid bounded event page").into());
    }
    let output_bytes = page
        .events()
        .iter()
        .filter(|event| event.kind() == TactusEventKind::Output)
        .filter_map(tactus_core::RunEvent::blob)
        .try_fold(0_usize, |total, blob| {
            usize::try_from(blob.length())
                .ok()
                .and_then(|length| total.checked_add(length))
        })
        .ok_or_else(|| io::Error::other("Tactus output length overflowed"))?;
    let result = TactusExecution {
        event_count: page.events().len(),
        output_bytes,
    };
    daemon.shutdown(Duration::from_secs(2))?;
    Ok(result)
}

struct LocalAgentrodPort {
    service: Agentrod<TactusBackend, RequiredOutputsGate>,
    plan_digest: Sha256Digest,
    workspace_id: WorkspaceId,
    runs: BTreeMap<String, OrchestrationRunId>,
    starts: usize,
}

impl LocalAgentrodPort {
    fn drive(&mut self, run_id: &OrchestrationRunId) -> TestResult<RunState> {
        let run_id = RunId::parse(run_id.as_str())?;
        for _ in 0..16 {
            let snapshot = self.service.advance_run(&run_id)?;
            if snapshot.state().is_terminal() {
                return Ok(snapshot.state());
            }
        }
        Err(io::Error::other("local agentrod did not reach a terminal state").into())
    }

    fn events(&self, run_id: &OrchestrationRunId, limit: usize) -> TestResult<Vec<u64>> {
        let run_id = RunId::parse(run_id.as_str())?;
        Ok(self
            .service
            .watch_run(&WatchRunRequest::new(run_id, 0, limit)?)?
            .events()
            .iter()
            .map(agentrod::RunEvent::sequence)
            .collect())
    }
}

impl AgentrodPort for LocalAgentrodPort {
    fn start_workflow(&mut self, request: &DispatchRequest) -> Result<DispatchStart, PortError> {
        if request.plan_digest != self.plan_digest {
            return Err(PortError::Rejected("PLAN_DIGEST_MISMATCH"));
        }
        if let Some(existing) = self.runs.get(request.occurrence_id.as_str()) {
            return Ok(DispatchStart::Accepted(existing.clone()));
        }
        let digest = request
            .occurrence_id
            .as_str()
            .strip_prefix("occ-sha256:")
            .ok_or(PortError::InvalidRequest)?;
        let run_id =
            RunId::parse(&format!("run-{digest}")).map_err(|_| PortError::InvalidRequest)?;
        self.service
            .start_run(StartRunRequest::new(
                RequestId::generate(),
                run_id.clone(),
                self.plan_digest,
                self.workspace_id.clone(),
            ))
            .map_err(|_| PortError::Rejected("AGENTROD_START_REJECTED"))?;
        let reference =
            OrchestrationRunId::parse(run_id.as_str()).map_err(|_| PortError::InvalidRequest)?;
        self.runs
            .insert(request.occurrence_id.as_str().to_owned(), reference.clone());
        self.starts += 1;
        Ok(DispatchStart::Accepted(reference))
    }

    fn query_by_occurrence(
        &mut self,
        occurrence_id: &segno_core::OccurrenceId,
    ) -> Result<DispatchLookup, PortError> {
        Ok(self
            .runs
            .get(occurrence_id.as_str())
            .cloned()
            .map_or(DispatchLookup::NotFound, DispatchLookup::Found))
    }
}

#[test]
fn python_fixture_runs_through_agentrod_tactus_and_segno_reference() -> TestResult {
    let fixture: Fixture = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../fixtures/cross-language/alpha-workflow.json"
    )))?;
    assert_eq!(fixture.fixture_version, "agentro.cross-language-fixture/v1");
    assert_eq!(fixture.release_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        (fixture.protocol.api_major, fixture.protocol.api_minor),
        (1, 0)
    );
    assert_eq!(
        fixture.protocol.workflow_proto,
        "agentro.workflow.v1.WorkflowDefinition"
    );
    assert_eq!(
        fixture
            .products
            .iter()
            .map(|product| product.distribution.as_str())
            .collect::<Vec<_>>(),
        ["clef-sdk", "tactus-runtime", "motivo-studio", "segno-flow"]
    );
    assert_eq!(
        fixture.products[0].python_import.as_deref(),
        Some("clef_sdk")
    );
    assert_eq!(fixture.products[1].entry_points, ["tactus"]);
    assert_eq!(
        fixture.products[3].entry_points,
        ["segno-flow", "segno-flow-ui"]
    );
    assert_eq!(fixture.expected.tactus_output.as_bytes(), TACTUS_OUTPUT);

    let wire_workflow = wire_workflow(&fixture.workflow)?;
    let encoded = wire_workflow.encode_to_vec();
    let decoded = wire::WorkflowDefinition::decode(encoded.as_slice())?;
    assert_eq!(decoded, wire_workflow);
    let workflow = domain_workflow(&decoded)?;

    let provider = ProviderName::parse("tactus-local")?;
    let capabilities =
        CapabilitySet::from_capabilities([AgentCapability::Streaming.capability()?])?;
    let report = ProbeReport::new(
        provider,
        env!("CARGO_PKG_VERSION"),
        ProtocolVersion::V1,
        capabilities,
        true,
    )?;
    let metrics = Arc::new(IntegrationMetrics::default());
    let backend = TactusBackend {
        report,
        metrics: Arc::clone(&metrics),
    };
    let mut service = Agentrod::new(
        backend,
        RequiredOutputsGate,
        ServiceLimits::new(4, 4, 64, fixture.expected.max_events as usize, 8)?,
    );
    let compiled = service.compile_workflow(CompileWorkflowRequest::new(
        RequestId::generate(),
        workflow,
        Sha256Digest::from_bytes([9; 32]),
    ))?;
    let mut port = LocalAgentrodPort {
        service,
        plan_digest: compiled.plan_digest(),
        workspace_id: WorkspaceId::parse(&fixture.expected.workspace_id)?,
        runs: BTreeMap::new(),
        starts: 0,
    };

    let mut occurrence = Occurrence::new(
        SegnoTaskId::parse("alpha-vertical-slice")?,
        ScheduleRevision::new(1)?,
        UtcInstant::from_millis(1_800_000_000_000),
    )?;
    let owner = LeaseOwnerId::parse("segnod-alpha")?;
    let fence = occurrence.claim(
        owner.clone(),
        UtcInstant::from_millis(1_800_000_000_000),
        30_000,
    )?;
    let dispatch = DispatchRequest {
        occurrence_id: occurrence.id.clone(),
        revision: occurrence.schedule_revision,
        plan_digest: compiled.plan_digest(),
        owner: owner.clone(),
        fencing_token: fence,
    };
    let first = port.start_workflow(&dispatch)?;
    let replay = port.start_workflow(&dispatch)?;
    assert_eq!(first, replay);
    assert_eq!(port.starts, 1);
    let DispatchStart::Accepted(run_reference) = first else {
        return Err(io::Error::other("local agentrod returned an unknown outcome").into());
    };
    occurrence.record_dispatch(&owner, fence, run_reference.clone())?;
    assert_eq!(
        port.query_by_occurrence(&occurrence.id)?,
        DispatchLookup::Found(run_reference.clone())
    );

    assert_eq!(port.drive(&run_reference)?, RunState::Succeeded);
    let sequences = port.events(&run_reference, fixture.expected.max_events as usize)?;
    assert!(sequences.len() <= fixture.expected.max_events as usize);
    assert_eq!(sequences, (1..=sequences.len() as u64).collect::<Vec<_>>());
    let run_id = RunId::parse(run_reference.as_str())?;
    let terminal = port.service.watch_run(&WatchRunRequest::new(
        run_id,
        0,
        fixture.expected.max_events as usize,
    )?)?;
    assert!(matches!(
        terminal.events().last().map(agentrod::RunEvent::body),
        Some(RunEventBody::RunSucceeded)
    ));
    assert_eq!(metrics.tactus_runs.load(Ordering::SeqCst), 1);
    assert!(metrics.tactus_events.load(Ordering::SeqCst) <= 32);
    assert_eq!(
        metrics.tactus_output_bytes.load(Ordering::SeqCst),
        TACTUS_OUTPUT.len()
    );
    assert_eq!(metrics.closed_sessions.load(Ordering::SeqCst), 1);
    Ok(())
}

fn wire_workflow(fixture: &WorkflowFixture) -> Result<wire::WorkflowDefinition, io::Error> {
    let tasks = fixture
        .tasks
        .iter()
        .map(|task| {
            if !task.inputs.is_empty() {
                return Err(io::Error::other(
                    "alpha fixture unexpectedly contains task inputs",
                ));
            }
            let prompts = task
                .prompts
                .iter()
                .map(|prompt| {
                    Ok(wire::Prompt {
                        role: prompt_role(&prompt.role)? as i32,
                        content: prompt.content.clone(),
                        name: prompt.name.clone(),
                        priority: prompt.priority,
                    })
                })
                .collect::<Result<Vec<_>, io::Error>>()?;
            let outputs = task
                .outputs
                .iter()
                .map(|output| {
                    Ok(wire::ArtifactSpec {
                        name: output.name.clone(),
                        description: output.description.clone(),
                        kind: artifact_kind(&output.kind)? as i32,
                        path: output.path.clone(),
                        required: output.required,
                    })
                })
                .collect::<Result<Vec<_>, io::Error>>()?;
            let effects = task
                .effects
                .iter()
                .map(|effect| {
                    Ok(wire::Effect {
                        kind: effect_kind(&effect.kind)? as i32,
                        path_glob: effect.path_glob.clone(),
                    })
                })
                .collect::<Result<Vec<_>, io::Error>>()?;
            Ok(wire::TaskDefinition {
                id: task.id.clone(),
                domain_function: task.domain_function.clone(),
                prompts,
                inputs: Vec::new(),
                outputs,
                effects,
                required_capabilities: task.required_capabilities.clone(),
                preferred_capabilities: task.preferred_capabilities.clone(),
                effort: task.effort.as_deref().map(effort).transpose()?,
            })
        })
        .collect::<Result<Vec<_>, io::Error>>()?;
    let outputs = fixture
        .outputs
        .iter()
        .map(|output| wire::WorkflowOutput {
            name: output.name.clone(),
            source: Some(wire::ArtifactBinding {
                source_task_id: output.source.source_task_id.clone(),
                output_name: output.source.output_name.clone(),
            }),
        })
        .collect();
    Ok(wire::WorkflowDefinition {
        schema_version: fixture.schema_version.clone(),
        id: fixture.id.clone(),
        tasks,
        outputs,
        policy: Some(wire::WorkflowPolicy {
            max_concurrency: fixture.policy.max_concurrency,
            fail_fast: fixture.policy.fail_fast,
            max_fan_out: fixture.policy.max_fan_out,
        }),
        required_capabilities: fixture.required_capabilities.clone(),
    })
}

fn domain_workflow(value: &wire::WorkflowDefinition) -> TestResult<WorkflowSpec> {
    if value.schema_version != "clef.workflow/v2" || !value.required_capabilities.is_empty() {
        return Err(io::Error::other("unsupported alpha workflow envelope").into());
    }
    let schema = SchemaVersion::V2;
    let mut tasks = Vec::with_capacity(value.tasks.len());
    for wire_task in &value.tasks {
        if !wire_task.inputs.is_empty() {
            return Err(io::Error::other("alpha task inputs are not supported").into());
        }
        let instruction = wire_task
            .prompts
            .iter()
            .find(|prompt| prompt.role == wire::PromptRole::Instruction as i32)
            .ok_or_else(|| io::Error::other("task instruction prompt is missing"))?;
        let mut task = TaskSpec::new(
            schema,
            TaskId::parse(&wire_task.id)?,
            DomainFunctionName::parse(&wire_task.domain_function)?,
            &instruction.content,
        )?;
        for output in &wire_task.outputs {
            let path = output
                .path
                .as_deref()
                .ok_or_else(|| io::Error::other("physical artifact path is missing"))?;
            task = task.with_output(ArtifactSpec::new(
                schema,
                ArtifactName::parse(&output.name)?,
                &output.description,
                domain_artifact_kind(output.kind)?,
                ProjectPath::parse(path)?,
                output.required,
            )?)?;
        }
        let effects = wire_task
            .effects
            .iter()
            .map(|effect| {
                Ok(EffectRule::new(
                    domain_effect_kind(effect.kind)?,
                    effect
                        .path_glob
                        .as_deref()
                        .map(ProjectPath::parse)
                        .transpose()?,
                ))
            })
            .collect::<TestResult<Vec<_>>>()?;
        task = task.with_effects(EffectPolicy::new(schema, effects)?);
        for capability in &wire_task.required_capabilities {
            task =
                task.requiring_capability(agentro_contracts::CapabilityName::parse(capability)?)?;
        }
        for capability in &wire_task.preferred_capabilities {
            task =
                task.preferring_capability(agentro_contracts::CapabilityName::parse(capability)?)?;
        }
        if wire_task.effort.is_some() {
            return Err(io::Error::other("alpha fixture unexpectedly selects effort").into());
        }
        tasks.push(task);
    }
    for output in &value.outputs {
        let source = output
            .source
            .as_ref()
            .ok_or_else(|| io::Error::other("workflow output source is missing"))?;
        if !value.tasks.iter().any(|task| {
            task.id == source.source_task_id
                && task
                    .outputs
                    .iter()
                    .any(|item| item.name == source.output_name)
        }) {
            return Err(io::Error::other("workflow output source does not exist").into());
        }
    }
    let policy = value
        .policy
        .as_ref()
        .ok_or_else(|| io::Error::other("workflow policy is missing"))?;
    Ok(WorkflowSpec::new(
        schema,
        WorkflowId::parse(&value.id)?,
        tasks,
        Vec::new(),
        WorkflowPolicy::new(
            schema,
            u16::try_from(policy.max_concurrency)?,
            u16::try_from(policy.max_fan_out)?,
            policy.fail_fast,
        )?,
    )?)
}

fn prompt_role(value: &str) -> Result<wire::PromptRole, io::Error> {
    match value {
        "policy" => Ok(wire::PromptRole::Policy),
        "context" => Ok(wire::PromptRole::Context),
        "instruction" => Ok(wire::PromptRole::Instruction),
        "repair" => Ok(wire::PromptRole::Repair),
        _ => Err(io::Error::other("unknown prompt role")),
    }
}

fn artifact_kind(value: &str) -> Result<wire::ArtifactKind, io::Error> {
    match value {
        "file" => Ok(wire::ArtifactKind::File),
        "directory" => Ok(wire::ArtifactKind::Directory),
        "text" => Ok(wire::ArtifactKind::Text),
        "json" => Ok(wire::ArtifactKind::Json),
        _ => Err(io::Error::other("unknown artifact kind")),
    }
}

fn effect_kind(value: &str) -> Result<wire::EffectKind, io::Error> {
    match value {
        "read" => Ok(wire::EffectKind::Read),
        "create" => Ok(wire::EffectKind::Create),
        "modify" => Ok(wire::EffectKind::Modify),
        "move" => Ok(wire::EffectKind::Move),
        "delete" => Ok(wire::EffectKind::Delete),
        "shell" => Ok(wire::EffectKind::Shell),
        "network" => Ok(wire::EffectKind::Network),
        _ => Err(io::Error::other("unknown effect kind")),
    }
}

fn effort(value: &str) -> Result<i32, io::Error> {
    match value {
        "xhigh" => Ok(wire::Effort::Xhigh as i32),
        "high" => Ok(wire::Effort::High as i32),
        "medium" => Ok(wire::Effort::Medium as i32),
        "low" => Ok(wire::Effort::Low as i32),
        _ => Err(io::Error::other("unknown effort")),
    }
}

fn domain_artifact_kind(value: i32) -> TestResult<ArtifactKind> {
    match wire::ArtifactKind::try_from(value)? {
        wire::ArtifactKind::File => Ok(ArtifactKind::File),
        wire::ArtifactKind::Directory => Ok(ArtifactKind::Directory),
        wire::ArtifactKind::Text => Ok(ArtifactKind::Text),
        wire::ArtifactKind::Json => Ok(ArtifactKind::Json),
        wire::ArtifactKind::Unspecified => {
            Err(io::Error::other("unspecified artifact kind").into())
        }
    }
}

fn domain_effect_kind(value: i32) -> TestResult<EffectKind> {
    match wire::EffectKind::try_from(value)? {
        wire::EffectKind::Read => Ok(EffectKind::Read),
        wire::EffectKind::Create => Ok(EffectKind::Create),
        wire::EffectKind::Modify => Ok(EffectKind::Modify),
        wire::EffectKind::Move => Ok(EffectKind::Move),
        wire::EffectKind::Delete => Ok(EffectKind::Delete),
        wire::EffectKind::Shell => Ok(EffectKind::Shell),
        wire::EffectKind::Network => Ok(EffectKind::Network),
        wire::EffectKind::Unspecified => Err(io::Error::other("unspecified effect kind").into()),
    }
}
