mod common;

use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use segno_core::{
    AgentrodPort, CompileWorkflowPort, CompileWorkflowRequest, CompileWorkflowResponse,
    DispatchLookup, DispatchRequest, DispatchStart, LeaseOwnerId, OccurrenceState,
    OrchestrationRunId, PortError, Sha256Digest, UtcInstant,
};
use segnod::{ArchiveBudget, SchedulerConfig, Segnod, SqliteStore, StaticCompiler, StoreError};

use common::{manifest, test_directory, write_package};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([byte; 32])
}

fn configured(capacity: usize, ttl_ms: u64) -> SchedulerConfig {
    SchedulerConfig {
        dispatch_capacity: capacity,
        lease_ttl: std::time::Duration::from_millis(ttl_ms),
        schedules_per_tick: 10,
        misfire_scan_limit: 100,
        misfire_output_limit: 10,
    }
}

fn imported_daemon(
    fixture: &tempfile::TempDir,
    overlap: &str,
    capacity: usize,
    ttl_ms: u64,
) -> Result<(Segnod, String), Box<dyn std::error::Error>> {
    let archive = write_package(
        &fixture.path().join("task.zip"),
        &manifest(overlap, "coalesce"),
        Vec::new(),
    )?;
    let root = fixture.path().join("state");
    let mut daemon = Segnod::open(
        &root,
        ArchiveBudget::default(),
        configured(capacity, ttl_ms),
    )?;
    let imported = daemon.import_package(&archive, UtcInstant::from_millis(60_001))?;
    let mut compiler = StaticCompiler::new(digest(9));
    daemon.enable(
        &imported.task_id,
        imported.revision,
        &mut compiler,
        UtcInstant::from_millis(60_001),
    )?;
    Ok((daemon, imported.task_id))
}

fn corrupt_current_revision(
    fixture: &tempfile::TempDir,
    task_id: &str,
    revision: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::open(fixture.path().join("state/segno.sqlite3"))?;
    connection.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA ignore_check_constraints = ON;")?;
    connection.execute(
        "UPDATE task_revisions SET revision = ?2 WHERE task_id = ?1",
        params![task_id, revision],
    )?;
    connection.execute(
        "UPDATE tasks SET current_revision = ?2 WHERE task_id = ?1",
        params![task_id, revision],
    )?;
    Ok(())
}

#[derive(Default)]
struct FakeAgentrod {
    runs: BTreeMap<String, OrchestrationRunId>,
    lose_next_response: bool,
    starts: usize,
    requests: Vec<DispatchRequest>,
}

impl AgentrodPort for FakeAgentrod {
    fn start_workflow(&mut self, request: &DispatchRequest) -> Result<DispatchStart, PortError> {
        self.starts += 1;
        self.requests.push(request.clone());
        let run = if let Some(run) = self.runs.get(request.occurrence_id.as_str()) {
            run.clone()
        } else {
            let run = OrchestrationRunId::parse(&format!("run-{}", self.runs.len() + 1))
                .map_err(|_| PortError::InvalidRequest)?;
            self.runs
                .insert(request.occurrence_id.as_str().to_owned(), run.clone());
            run
        };
        if self.lose_next_response {
            self.lose_next_response = false;
            Ok(DispatchStart::OutcomeUnknown)
        } else {
            Ok(DispatchStart::Accepted(run))
        }
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
fn response_loss_reconciles_after_restart_without_second_run() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, task_id) = imported_daemon(&fixture, "forbid", 1, 10)?;
    let occurrence = daemon.run_now(&task_id, UtcInstant::from_millis(70_000))?;
    let mut agentrod = FakeAgentrod {
        lose_next_response: true,
        ..FakeAgentrod::default()
    };
    let first = daemon.dispatch_once("owner-a", &mut agentrod, UtcInstant::from_millis(70_000))?;
    assert_eq!(first.claimed, 1);
    assert_eq!(first.unknown, 1);
    assert_eq!(
        daemon.status(occurrence.as_str())?.state,
        OccurrenceState::Dispatching
    );
    daemon.shutdown()?;

    let root = fixture.path().join("state");
    let mut restarted = Segnod::open(&root, ArchiveBudget::default(), configured(1, 10))?;
    let recovered =
        restarted.reconcile_once("owner-b", &mut agentrod, UtcInstant::from_millis(70_011))?;
    assert_eq!(recovered.accepted, 1);
    assert_eq!(agentrod.starts, 1, "query found the first accepted run");
    let status = restarted.status(occurrence.as_str())?;
    assert_eq!(status.state, OccurrenceState::Dispatched);
    assert_eq!(status.orchestration_run_id.as_deref(), Some("run-1"));
    restarted.shutdown()?;
    Ok(())
}

#[test]
fn stale_owner_fence_cannot_overwrite_new_owner() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, task_id) = imported_daemon(&fixture, "forbid", 1, 10)?;
    daemon.run_now(&task_id, UtcInstant::from_millis(80_000))?;
    daemon.shutdown()?;

    let mut store = SqliteStore::open(&fixture.path().join("state/segno.sqlite3"))?;
    let old_owner = LeaseOwnerId::parse("owner-a")?;
    let old = store.claim_due(&old_owner, UtcInstant::from_millis(80_000), 10, 1)?;
    assert_eq!(old.len(), 1);
    let new_owner = LeaseOwnerId::parse("owner-b")?;
    let current = store.claim_reconciliation(&new_owner, UtcInstant::from_millis(80_011), 10, 1)?;
    assert_eq!(current.len(), 1);
    assert!(current[0].fencing_token > old[0].fencing_token);
    let run = OrchestrationRunId::parse("run-current")?;
    assert!(matches!(
        store.record_dispatch(&old[0], &run, UtcInstant::from_millis(80_012)),
        Err(StoreError::FenceRejected)
    ));
    store.record_dispatch(&current[0], &run, UtcInstant::from_millis(80_012))?;
    store.checkpoint()?;
    Ok(())
}

#[test]
fn expired_lease_rejects_accepted_dispatch() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, task_id) = imported_daemon(&fixture, "forbid", 1, 10)?;
    let occurrence = daemon.run_now(&task_id, UtcInstant::from_millis(85_000))?;
    daemon.shutdown()?;

    let mut store = SqliteStore::open(&fixture.path().join("state/segno.sqlite3"))?;
    let owner = LeaseOwnerId::parse("owner-a")?;
    let claims = store.claim_due(&owner, UtcInstant::from_millis(85_000), 10, 1)?;
    assert_eq!(claims.len(), 1);
    let run = OrchestrationRunId::parse("run-expired")?;
    assert!(matches!(
        store.record_dispatch(&claims[0], &run, UtcInstant::from_millis(85_011)),
        Err(StoreError::FenceRejected)
    ));
    let status = store.occurrence_status(&occurrence)?;
    assert_eq!(status.state, OccurrenceState::Dispatching);
    assert_eq!(status.orchestration_run_id, None);
    store.checkpoint()?;
    Ok(())
}

#[test]
fn exactly_expired_lease_rejects_known_dispatch_failure() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, task_id) = imported_daemon(&fixture, "forbid", 1, 10)?;
    let occurrence = daemon.run_now(&task_id, UtcInstant::from_millis(86_000))?;
    daemon.shutdown()?;

    let mut store = SqliteStore::open(&fixture.path().join("state/segno.sqlite3"))?;
    let owner = LeaseOwnerId::parse("owner-a")?;
    let claims = store.claim_due(&owner, UtcInstant::from_millis(86_000), 10, 1)?;
    assert_eq!(claims.len(), 1);
    assert!(matches!(
        store.record_dispatch_failure(
            &claims[0],
            "agentrod_unavailable",
            UtcInstant::from_millis(86_010),
        ),
        Err(StoreError::FenceRejected)
    ));
    let status = store.occurrence_status(&occurrence)?;
    assert_eq!(status.state, OccurrenceState::Dispatching);
    assert_eq!(status.summary_code, None);
    store.checkpoint()?;
    Ok(())
}

#[test]
fn list_tasks_rejects_negative_current_revision_as_corrupt() -> TestResult {
    let fixture = test_directory()?;
    let (daemon, task_id) = imported_daemon(&fixture, "forbid", 1, 10)?;
    daemon.shutdown()?;
    let store = SqliteStore::open(&fixture.path().join("state/segno.sqlite3"))?;
    corrupt_current_revision(&fixture, &task_id, -1)?;

    assert!(matches!(
        store.list_tasks(None, 10),
        Err(StoreError::CorruptValue("revision"))
    ));
    Ok(())
}

#[test]
fn list_tasks_rejects_zero_current_revision_as_corrupt() -> TestResult {
    let fixture = test_directory()?;
    let (daemon, task_id) = imported_daemon(&fixture, "forbid", 1, 10)?;
    daemon.shutdown()?;
    let store = SqliteStore::open(&fixture.path().join("state/segno.sqlite3"))?;
    corrupt_current_revision(&fixture, &task_id, 0)?;

    assert!(matches!(
        store.list_tasks(None, 10),
        Err(StoreError::CorruptValue("revision"))
    ));
    Ok(())
}

#[test]
fn occurrence_identity_and_dispatch_admission_are_bounded() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, task_id) = imported_daemon(&fixture, "allow", 2, 100)?;
    let first = daemon.run_now(&task_id, UtcInstant::from_millis(90_000))?;
    let duplicate = daemon.run_now(&task_id, UtcInstant::from_millis(90_000))?;
    assert_eq!(first, duplicate);
    let second = daemon.run_now(&task_id, UtcInstant::from_millis(90_001))?;
    let third = daemon.run_now(&task_id, UtcInstant::from_millis(90_002))?;
    let mut agentrod = FakeAgentrod::default();
    let batch = daemon.dispatch_once("owner", &mut agentrod, UtcInstant::from_millis(90_002))?;
    assert_eq!(batch.claimed, 2);
    assert_eq!(batch.accepted, 2);
    assert_eq!(
        daemon.status(third.as_str())?.state,
        OccurrenceState::Queued
    );
    assert_eq!(
        daemon.status(first.as_str())?.state,
        OccurrenceState::Dispatched
    );
    assert_eq!(
        daemon.status(second.as_str())?.state,
        OccurrenceState::Dispatched
    );
    daemon.shutdown()?;
    Ok(())
}

#[test]
fn terminal_summary_releases_forbid_overlap_without_copying_outputs() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, task_id) = imported_daemon(&fixture, "forbid", 2, 100)?;
    let first = daemon.run_now(&task_id, UtcInstant::from_millis(95_000))?;
    let second = daemon.run_now(&task_id, UtcInstant::from_millis(95_001))?;
    let mut agentrod = FakeAgentrod::default();
    let initial = daemon.dispatch_once("owner", &mut agentrod, UtcInstant::from_millis(95_001))?;
    assert_eq!(initial.claimed, 1);
    assert_eq!(
        daemon.status(second.as_str())?.state,
        OccurrenceState::Queued
    );

    daemon.record_terminal_summary(
        first.as_str(),
        true,
        "workflow_succeeded",
        UtcInstant::from_millis(95_010),
    )?;
    assert_eq!(
        daemon.status(first.as_str())?.state,
        OccurrenceState::Succeeded
    );
    let next = daemon.dispatch_once("owner", &mut agentrod, UtcInstant::from_millis(95_011))?;
    assert_eq!(next.claimed, 1);
    assert_eq!(
        daemon.status(second.as_str())?.state,
        OccurrenceState::Dispatched
    );
    daemon.shutdown()?;
    Ok(())
}

#[test]
fn cron_tick_coalesces_downtime_to_one_unique_occurrence() -> TestResult {
    let fixture = test_directory()?;
    let (mut daemon, _task_id) = imported_daemon(&fixture, "forbid", 2, 100)?;
    let inserted = daemon.tick(UtcInstant::from_millis(300_000))?;
    assert_eq!(inserted.len(), 1);
    let status = daemon.status(inserted[0].as_str())?;
    assert_eq!(status.scheduled_for_ms, 300_000);
    assert_eq!(status.state, OccurrenceState::Queued);
    daemon.shutdown()?;
    Ok(())
}

struct RecordingCompiler {
    calls: usize,
}

impl CompileWorkflowPort for RecordingCompiler {
    fn compile_workflow(
        &mut self,
        request: &CompileWorkflowRequest,
    ) -> Result<CompileWorkflowResponse, PortError> {
        self.calls += 1;
        if request
            .workflow_spec()
            .starts_with(b"segno-clef-execution-spec-v1\0")
        {
            Ok(CompileWorkflowResponse {
                plan_digest: digest(7),
            })
        } else {
            Err(PortError::InvalidRequest)
        }
    }
}

#[test]
fn enable_delegates_frozen_stage_spec_to_compile_port() -> TestResult {
    let fixture = test_directory()?;
    let archive = write_package(
        &fixture.path().join("task.zip"),
        &manifest("forbid", "coalesce"),
        Vec::new(),
    )?;
    let root = fixture.path().join("state");
    let mut daemon = Segnod::open(&root, ArchiveBudget::default(), configured(1, 100))?;
    let imported = daemon.import_package(&archive, UtcInstant::from_millis(1_000))?;
    let mut compiler = RecordingCompiler { calls: 0 };
    let plan = daemon.enable(
        &imported.task_id,
        imported.revision,
        &mut compiler,
        UtcInstant::from_millis(1_000),
    )?;
    assert_eq!(compiler.calls, 1);
    assert_eq!(plan, digest(7));
    assert!(daemon.list_tasks(None, 10)?.tasks[0].enabled);
    daemon.shutdown()?;
    Ok(())
}
