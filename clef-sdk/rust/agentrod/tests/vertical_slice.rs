use agentro_contracts::{CapabilityName, CapabilitySet, RequestId, Sha256Digest, UtcTimestamp};
use agentrod::{
    Agentrod, AgentrodError, CancelRunRequest, CompileWorkflowRequest, RunEventBody, RunFailure,
    RunState, ServiceLimits, StartRunRequest, WatchRunRequest, derive_session_coordinates,
};
use clef_agent::{
    AgentCapability, AgentEvent, AgentEventBody, AgentEventValidator, AgentProtocolError,
    AgentSessionId, AgentTurnId, FakeBackend, ProbeReport, ProtocolVersion, ProviderName,
    WorkspaceId,
};
use clef_core::{
    ArtifactBinding, ArtifactKind, ArtifactName, ArtifactSpec, Attempt, AttemptCommand,
    AttemptNumber, AttemptState, CompileContext, CompileError, CompileIssueCode,
    DomainFunctionName, Effort, ProjectPath, ReadyScheduler, RequiredOutputsGate, RunId,
    SchemaVersion, TaskId, TaskOutcome, TaskScheduleState, TaskSpec, WorkflowId, WorkflowPolicy,
    WorkflowSpec, compile_workflow,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn task(id: &str, instruction: &str) -> Result<TaskSpec, Box<dyn std::error::Error>> {
    Ok(TaskSpec::new(
        SchemaVersion::V1,
        TaskId::parse(id)?,
        DomainFunctionName::parse("test.work")?,
        instruction,
    )?)
}

fn output(
    name: &str,
    path: &str,
    kind: ArtifactKind,
) -> Result<ArtifactSpec, Box<dyn std::error::Error>> {
    Ok(ArtifactSpec::new(
        SchemaVersion::V1,
        ArtifactName::parse(name)?,
        "test output",
        kind,
        ProjectPath::parse(path)?,
        true,
    )?)
}

fn binding(
    source: &str,
    output_name: &str,
    target: &str,
    input_name: &str,
) -> Result<ArtifactBinding, Box<dyn std::error::Error>> {
    Ok(ArtifactBinding::new(
        TaskId::parse(source)?,
        ArtifactName::parse(output_name)?,
        TaskId::parse(target)?,
        ArtifactName::parse(input_name)?,
    ))
}

fn three_task_workflow(
    max_concurrency: u16,
    reverse_bindings: bool,
) -> Result<WorkflowSpec, Box<dyn std::error::Error>> {
    let alpha = task("alpha", "produce alpha")?.with_output(output(
        "left",
        "out/left.txt",
        ArtifactKind::Text,
    )?)?;
    let beta = task("beta", "produce beta")?.with_output(output(
        "right",
        "out/right.txt",
        ArtifactKind::Text,
    )?)?;
    let merge = task("merge", "merge both inputs")?
        .with_input(ArtifactName::parse("left")?, ArtifactKind::Text)?
        .with_input(ArtifactName::parse("right")?, ArtifactKind::Text)?
        .with_output(output("final", "out/final.txt", ArtifactKind::Text)?)?;
    let mut bindings = vec![
        binding("alpha", "left", "merge", "left")?,
        binding("beta", "right", "merge", "right")?,
    ];
    if reverse_bindings {
        bindings.reverse();
    }
    Ok(WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("deterministic-plan")?,
        vec![alpha, beta, merge],
        bindings,
        WorkflowPolicy::new(SchemaVersion::V1, max_concurrency, 8, true)?,
    )?)
}

fn compile_context(capabilities: CapabilitySet) -> CompileContext {
    CompileContext::new(capabilities, Sha256Digest::from_bytes([7; 32]))
}

#[test]
fn dag_plan_and_digest_are_deterministic() -> TestResult {
    let first = compile_workflow(
        &three_task_workflow(2, false)?,
        &compile_context(CapabilitySet::default()),
    )?;
    let reordered_bindings = compile_workflow(
        &three_task_workflow(2, true)?,
        &compile_context(CapabilitySet::default()),
    )?;

    let order: Vec<&str> = first
        .tasks()
        .iter()
        .map(|item| item.spec().id().as_str())
        .collect();
    assert_eq!(order, ["alpha", "beta", "merge"]);
    assert_eq!(first.digest(), reordered_bindings.digest());
    assert_eq!(first.ready_sets().len(), 2);
    assert_eq!(first.ready_sets()[0].tasks().len(), 2);
    assert_eq!(first.ready_sets()[1].tasks()[0].as_str(), "merge");

    let changed = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("deterministic-plan")?,
        vec![task("alpha", "changed instruction")?],
        Vec::new(),
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let changed = compile_workflow(&changed, &compile_context(CapabilitySet::default()))?;
    assert_ne!(first.digest(), changed.digest());
    Ok(())
}

#[test]
fn cycle_and_missing_artifact_are_typed_compile_failures() -> TestResult {
    let alpha = task("alpha", "alpha")?.with_dependency(TaskId::parse("beta")?)?;
    let beta = task("beta", "beta")?.with_dependency(TaskId::parse("alpha")?)?;
    let cyclic = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("cycle")?,
        vec![alpha, beta],
        Vec::new(),
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let error = compile_workflow(&cyclic, &compile_context(CapabilitySet::default()))
        .expect_err("cycle must fail");
    let CompileError::Validation(report) = error else {
        panic!("expected validation error");
    };
    let cycle = report
        .issues()
        .iter()
        .find(|issue| issue.code() == CompileIssueCode::Cycle)
        .expect("cycle issue");
    assert_eq!(cycle.related_tasks().first(), cycle.related_tasks().last());

    let producer = task("producer", "produce")?.with_output(output(
        "value",
        "out/value.json",
        ArtifactKind::Json,
    )?)?;
    let consumer = task("consumer", "consume")?
        .with_input(ArtifactName::parse("value")?, ArtifactKind::Text)?;
    let invalid = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("invalid-binding")?,
        vec![producer, consumer],
        vec![binding("producer", "missing", "consumer", "value")?],
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let error = compile_workflow(&invalid, &compile_context(CapabilitySet::default()))
        .expect_err("missing output must fail");
    let CompileError::Validation(report) = error else {
        panic!("expected validation error");
    };
    assert!(
        report
            .issues()
            .iter()
            .any(|issue| issue.code() == CompileIssueCode::MissingSourceOutput)
    );
    Ok(())
}

#[test]
fn strict_paths_effort_and_capabilities_remain_independent() -> TestResult {
    assert_eq!(
        ProjectPath::parse("src\\package\\main.rs")?.as_str(),
        "src/package/main.rs"
    );
    for path in [
        "../escape",
        "C:drive-relative",
        "//server/share",
        ".tactus/state",
        "out/CON.txt",
    ] {
        assert!(ProjectPath::parse(path).is_err(), "{path}");
    }

    let effort_task = task("routed", "work")?
        .with_effort(Effort::Xhigh)
        .preferring_capability(CapabilityName::parse(AgentCapability::Usage.as_str())?)?;
    let workflow = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("effort")?,
        vec![effort_task],
        Vec::new(),
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let error = compile_workflow(&workflow, &compile_context(CapabilitySet::default()))
        .expect_err("effort needs negotiated capability");
    let CompileError::Validation(report) = error else {
        panic!("expected validation error");
    };
    assert_eq!(
        report.issues()[0].code(),
        CompileIssueCode::CapabilityMissing
    );
    assert_eq!(report.issues()[0].subject(), Some("agent.reasoning-effort"));

    let capabilities =
        CapabilitySet::from_capabilities([AgentCapability::ReasoningEffort.capability()?])?;
    let plan = compile_workflow(&workflow, &compile_context(capabilities))?;
    assert_eq!(plan.tasks()[0].spec().effort(), Some(Effort::Xhigh));
    assert_eq!(
        plan.tasks()[0].missing_preferred_capabilities()[0].as_str(),
        "agent.usage"
    );
    Ok(())
}

#[test]
fn ready_scheduler_never_exceeds_policy_and_unlocks_in_order() -> TestResult {
    let plan = compile_workflow(
        &three_task_workflow(1, false)?,
        &compile_context(CapabilitySet::default()),
    )?;
    let mut scheduler = ReadyScheduler::new(&plan);

    let first = scheduler.admit_ready();
    assert_eq!(first[0].as_str(), "alpha");
    assert_eq!(scheduler.running_count(), 1);
    assert!(scheduler.admit_ready().is_empty());
    scheduler.finish(&first[0], TaskOutcome::Succeeded)?;

    let second = scheduler.admit_ready();
    assert_eq!(second[0].as_str(), "beta");
    scheduler.finish(&second[0], TaskOutcome::Succeeded)?;
    let third = scheduler.admit_ready();
    assert_eq!(third[0].as_str(), "merge");
    assert_eq!(scheduler.state(&third[0]), Some(TaskScheduleState::Running));
    scheduler.finish(&third[0], TaskOutcome::Succeeded)?;
    assert!(scheduler.workflow_state().is_terminal());
    Ok(())
}

#[test]
fn attempt_success_must_cross_verification_and_publish() -> TestResult {
    let mut attempt = Attempt::new(TaskId::parse("work")?, AttemptNumber::new(1)?);
    attempt.apply(AttemptCommand::Begin {
        approval_required: false,
    })?;
    assert!(attempt.apply(AttemptCommand::Complete).is_err());
    assert_eq!(attempt.state(), AttemptState::Running);
    attempt.apply(AttemptCommand::BeginVerification)?;
    attempt.apply(AttemptCommand::BeginPublishing)?;
    attempt.apply(AttemptCommand::Complete)?;
    assert_eq!(attempt.state(), AttemptState::Succeeded);
    assert!(attempt.apply(AttemptCommand::Cancel).is_err());
    assert_eq!(attempt.state(), AttemptState::Succeeded);
    Ok(())
}

fn event(
    provider: &ProviderName,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    sequence: u64,
    body: AgentEventBody,
) -> Result<AgentEvent, Box<dyn std::error::Error>> {
    Ok(AgentEvent::new(
        ProtocolVersion::V1,
        session_id.clone(),
        turn_id.clone(),
        sequence,
        UtcTimestamp::new(1_700_000_000 + sequence as i64, 0)?,
        provider.clone(),
        body,
    )?)
}

#[test]
fn normalized_event_validator_enforces_progress_order_and_unique_terminal() -> TestResult {
    let provider = ProviderName::parse("fake")?;
    let session_id = AgentSessionId::parse("session")?;
    let turn_id = AgentTurnId::parse("turn")?;
    let mut validator = AgentEventValidator::new(
        session_id.clone(),
        turn_id.clone(),
        provider.clone(),
        1024,
        4,
    )?;
    let first = event(
        &provider,
        &session_id,
        &turn_id,
        1,
        AgentEventBody::SessionStarted,
    )?;
    validator.accept(&first)?;
    let terminal = event(
        &provider,
        &session_id,
        &turn_id,
        2,
        AgentEventBody::TurnCompleted {
            artifacts: Vec::new(),
        },
    )?;
    validator.accept(&terminal)?;
    assert!(validator.is_terminal());
    let late = event(
        &provider,
        &session_id,
        &turn_id,
        3,
        AgentEventBody::Diagnostic {
            code: clef_agent::AgentDiagnosticCode::AdapterWarning,
            message: "late".into(),
        },
    )?;
    assert_eq!(
        validator.accept(&late),
        Err(AgentProtocolError::EventAfterTerminal)
    );
    Ok(())
}

fn report(provider: &ProviderName) -> Result<ProbeReport, Box<dyn std::error::Error>> {
    Ok(ProbeReport::new(
        provider.clone(),
        "1.0.0",
        ProtocolVersion::V1,
        CapabilitySet::from_capabilities([AgentCapability::Streaming.capability()?])?,
        true,
    )?)
}

fn success_script(
    run_id: &RunId,
    task_id: &str,
    provider: &ProviderName,
    output_name: &str,
    kind: ArtifactKind,
) -> Result<Vec<AgentEvent>, Box<dyn std::error::Error>> {
    let task_id = TaskId::parse(task_id)?;
    let coordinates = derive_session_coordinates(run_id, &task_id)?;
    Ok(vec![
        event(
            provider,
            coordinates.session_id(),
            coordinates.turn_id(),
            1,
            AgentEventBody::SessionStarted,
        )?,
        event(
            provider,
            coordinates.session_id(),
            coordinates.turn_id(),
            2,
            AgentEventBody::TurnCompleted {
                artifacts: vec![clef_core::ProducedArtifact::new(
                    ArtifactName::parse(output_name)?,
                    kind,
                )],
            },
        )?,
    ])
}

fn compile_in_service(
    service: &mut Agentrod<FakeBackend, RequiredOutputsGate>,
    workflow: WorkflowSpec,
) -> Result<Sha256Digest, Box<dyn std::error::Error>> {
    let request = CompileWorkflowRequest::new(
        RequestId::generate(),
        workflow,
        Sha256Digest::from_bytes([9; 32]),
    );
    let response = service.compile_workflow(request.clone())?;
    assert_eq!(service.compile_workflow(request)?, response);
    Ok(response.plan_digest())
}

#[test]
fn agentrod_compile_start_get_watch_and_publish_vertical_slice() -> TestResult {
    let run_id = RunId::parse("run-success")?;
    let provider = ProviderName::parse("fake")?;
    let scripts = vec![
        success_script(&run_id, "alpha", &provider, "left", ArtifactKind::Text)?,
        success_script(&run_id, "beta", &provider, "right", ArtifactKind::Text)?,
        success_script(&run_id, "merge", &provider, "final", ArtifactKind::Text)?,
    ];
    let backend = FakeBackend::new(report(&provider)?, scripts)?;
    let metrics = backend.metrics();
    let mut service = Agentrod::new(backend, RequiredOutputsGate, ServiceLimits::default());
    let plan_digest = compile_in_service(&mut service, three_task_workflow(2, false)?)?;
    let start = StartRunRequest::new(
        RequestId::generate(),
        run_id.clone(),
        plan_digest,
        WorkspaceId::parse("workspace")?,
    );
    let started = service.start_run(start.clone())?;
    assert_eq!(started.state(), RunState::Running);
    assert_eq!(service.start_run(start)?, started);

    let mut snapshot = started;
    for _ in 0..8 {
        snapshot = service.advance_run(&run_id)?;
        if snapshot.state().is_terminal() {
            break;
        }
    }
    assert_eq!(snapshot.state(), RunState::Succeeded);
    assert_eq!(service.get_run(&run_id)?, snapshot);
    assert_eq!(metrics.opened_sessions(), 3);
    assert_eq!(metrics.sent_turns(), 3);
    assert_eq!(metrics.closed_sessions(), 3);

    let mut sequences = Vec::new();
    let mut after = 0;
    loop {
        let page = service.watch_run(&WatchRunRequest::new(run_id.clone(), after, 4)?)?;
        sequences.extend(page.events().iter().map(agentrod::RunEvent::sequence));
        after = sequences.last().copied().unwrap_or_default();
        if !page.has_more() {
            assert_eq!(page.last_sequence(), snapshot.last_sequence());
            break;
        }
    }
    assert_eq!(
        sequences,
        (1..=snapshot.last_sequence()).collect::<Vec<_>>()
    );
    let all = service.watch_run(&WatchRunRequest::new(run_id.clone(), 0, 256)?)?;
    assert!(matches!(all.events()[0].body(), RunEventBody::RunStarted));
    assert!(matches!(
        all.events().last().map(agentrod::RunEvent::body),
        Some(RunEventBody::RunSucceeded)
    ));
    Ok(())
}

#[test]
fn agentrod_cancel_is_idempotent_and_closes_the_owned_session() -> TestResult {
    let run_id = RunId::parse("run-cancel")?;
    let provider = ProviderName::parse("fake")?;
    let coordinates = derive_session_coordinates(&run_id, &TaskId::parse("work")?)?;
    let backend = FakeBackend::new(
        report(&provider)?,
        vec![vec![event(
            &provider,
            coordinates.session_id(),
            coordinates.turn_id(),
            1,
            AgentEventBody::ContentDelta {
                text: "still running".into(),
            },
        )?]],
    )?;
    let metrics = backend.metrics();
    let mut service = Agentrod::new(backend, RequiredOutputsGate, ServiceLimits::default());
    let workflow = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("cancel-plan")?,
        vec![task("work", "wait")?],
        Vec::new(),
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let digest = compile_in_service(&mut service, workflow)?;
    service.start_run(StartRunRequest::new(
        RequestId::generate(),
        run_id.clone(),
        digest,
        WorkspaceId::parse("workspace")?,
    ))?;
    let cancel = CancelRunRequest::new(RequestId::generate(), run_id.clone());
    let first = service.cancel_run(cancel.clone())?;
    let second = service.cancel_run(cancel)?;
    let second_request_id = RequestId::generate();
    let third = service.cancel_run(CancelRunRequest::new(second_request_id, run_id.clone()))?;

    assert_eq!(first, second);
    assert_eq!(second, third);
    assert_eq!(first.state(), RunState::Cancelled);
    assert_eq!(metrics.cancelled_sessions(), 1);
    assert_eq!(metrics.closed_sessions(), 1);
    assert!(matches!(
        service.cancel_run(CancelRunRequest::new(
            second_request_id,
            RunId::parse("another-run")?,
        )),
        Err(AgentrodError::IdempotencyKeyReused)
    ));
    for _ in 0..2 {
        service.cancel_run(CancelRunRequest::new(RequestId::generate(), run_id.clone()))?;
    }
    assert!(matches!(
        service.cancel_run(CancelRunRequest::new(RequestId::generate(), run_id)),
        Err(AgentrodError::CancelRequestCapacityExhausted)
    ));
    Ok(())
}

#[test]
fn publish_gate_rejects_missing_required_output_before_success() -> TestResult {
    let run_id = RunId::parse("run-publish-reject")?;
    let provider = ProviderName::parse("fake")?;
    let task_id = TaskId::parse("work")?;
    let coordinates = derive_session_coordinates(&run_id, &task_id)?;
    let backend = FakeBackend::new(
        report(&provider)?,
        vec![vec![event(
            &provider,
            coordinates.session_id(),
            coordinates.turn_id(),
            1,
            AgentEventBody::TurnCompleted {
                artifacts: Vec::new(),
            },
        )?]],
    )?;
    let mut service = Agentrod::new(backend, RequiredOutputsGate, ServiceLimits::default());
    let workflow = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("publish-reject")?,
        vec![task("work", "produce")?.with_output(output(
            "required",
            "out/required.txt",
            ArtifactKind::Text,
        )?)?],
        Vec::new(),
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let digest = compile_in_service(&mut service, workflow)?;
    service.start_run(StartRunRequest::new(
        RequestId::generate(),
        run_id.clone(),
        digest,
        WorkspaceId::parse("workspace")?,
    ))?;
    let snapshot = service.advance_run(&run_id)?;
    assert_eq!(snapshot.state(), RunState::Failed);
    let events = service
        .watch_run(&WatchRunRequest::new(run_id, 0, 256)?)?
        .events()
        .to_vec();
    assert!(events.iter().any(|event| matches!(
        event.body(),
        RunEventBody::TaskFailed {
            failure: RunFailure::Publish(
                clef_core::PublishRejection::MissingRequiredArtifact { .. }
            ),
            ..
        }
    )));
    Ok(())
}

#[test]
fn output_paths_are_case_insensitively_unique() -> TestResult {
    let alpha = task("alpha", "alpha")?.with_output(output(
        "first",
        "Out/Report.txt",
        ArtifactKind::Text,
    )?)?;
    let beta = task("beta", "beta")?.with_output(output(
        "second",
        "out/report.txt",
        ArtifactKind::Text,
    )?)?;
    let workflow = WorkflowSpec::new(
        SchemaVersion::V1,
        WorkflowId::parse("paths")?,
        vec![alpha, beta],
        Vec::new(),
        WorkflowPolicy::new(SchemaVersion::V1, 1, 8, true)?,
    )?;
    let error = compile_workflow(&workflow, &compile_context(CapabilitySet::default()))
        .expect_err("duplicate portable paths must fail");
    let CompileError::Validation(report) = error else {
        panic!("expected validation error");
    };
    assert!(
        report
            .issues()
            .iter()
            .any(|issue| issue.code() == CompileIssueCode::DuplicateOutputPath)
    );
    Ok(())
}
