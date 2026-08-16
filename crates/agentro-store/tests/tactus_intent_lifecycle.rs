use std::{error::Error, path::Path, str::FromStr, time::Duration};

use agentro_contracts::Sha256Digest;
use agentro_store::{
    JournalMode, StoreConfig,
    tactus::{
        model::{
            BeginIntent, BlobRef, CellKey, CellState, CheckpointBackend, CheckpointEntry,
            CheckpointEntryKind, CheckpointKey, CheckpointRecord, ProjectKey, RollbackFidelity,
            RunState, TransactionState,
        },
        repository::{Repository, RepositoryError, RepositoryOwner},
    },
};
use rusqlite::Connection;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn config() -> Result<StoreConfig, agentro_store::StoreError> {
    StoreConfig::new(32, Duration::from_millis(250), JournalMode::Wal)
}

fn open_owner(database: &Path) -> Result<RepositoryOwner, RepositoryError> {
    RepositoryOwner::open(database.to_path_buf(), config()?, Duration::from_secs(2))
}

fn uuid_v7<T>(value: u64) -> Result<T, Box<dyn Error>>
where
    T: FromStr,
    T::Err: Error + 'static,
{
    Ok(format!("01890f3c-7b1d-7000-8000-{value:012x}").parse()?)
}

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([byte; 32])
}

fn intent(
    seed: u64,
    project_id: ProjectKey,
    cell_id: CellKey,
    source_byte: u8,
    now_ms: u64,
    expires_at_ms: u64,
) -> Result<BeginIntent, Box<dyn Error>> {
    Ok(BeginIntent {
        request_id: uuid_v7(seed * 10 + 1)?,
        request_digest: digest(source_byte),
        run_id: uuid_v7(seed * 10 + 2)?,
        transaction_id: uuid_v7(seed * 10 + 3)?,
        project_id,
        cell_id,
        source: BlobRef {
            digest: digest(source_byte),
            length: u64::from(source_byte),
        },
        workspace_binding: digest(240),
        owner_id: uuid_v7(seed * 10 + 4)?,
        now_ms,
        expires_at_ms,
    })
}

fn checkpoint(byte: u8, created_at_ms: u64) -> CheckpointRecord {
    let object = BlobRef {
        digest: digest(byte),
        length: u64::from(byte),
    };
    CheckpointRecord {
        id: CheckpointKey::from_digest(digest(byte)),
        manifest: object,
        backend: CheckpointBackend::NonGit,
        fidelity: RollbackFidelity::FullManifest,
        git_context: None,
        entries: vec![CheckpointEntry {
            path: "file.txt".to_owned(),
            kind: CheckpointEntryKind::File,
            object,
            is_executable: false,
        }],
        total_file_bytes: object.length,
        created_at_ms,
    }
}

fn repository_error<T>(
    result: Result<T, RepositoryError>,
) -> Result<RepositoryError, Box<dyn Error>> {
    match result {
        Ok(_) => Err("repository operation unexpectedly succeeded".into()),
        Err(error) => Ok(error),
    }
}

fn replay(
    repository: &Repository,
    input: &BeginIntent,
) -> Result<agentro_store::tactus::model::RunRecord, RepositoryError> {
    let result = repository.begin_intent(input.clone())?;
    if !result.replayed {
        return Err(RepositoryError::InvalidTransition);
    }
    Ok(result.run)
}

#[test]
fn begin_replays_the_original_run_and_rejects_changed_request_digest() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("replay.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let input = intent(1, uuid_v7(1)?, uuid_v7(2)?, 7, 10, 100)?;

    let created = repository.begin_intent(input.clone())?;
    assert!(!created.replayed);
    assert_eq!(created.run.run_id, input.run_id);
    assert_eq!(created.run.state, RunState::Pending);
    assert_eq!(created.run.cell_state, CellState::Queued);
    assert_eq!(created.run.transaction_state, TransactionState::Prepared);
    assert_eq!(created.run.last_sequence, 1);
    assert_eq!(created.run.revision, 1);

    let mut retry = input.clone();
    retry.run_id = uuid_v7(90)?;
    retry.transaction_id = uuid_v7(91)?;
    retry.owner_id = uuid_v7(92)?;
    let replayed = repository.begin_intent(retry)?;
    assert!(replayed.replayed);
    assert_eq!(replayed.run, created.run);
    assert_eq!(replayed.lease, created.lease);

    let mut conflicting = input;
    conflicting.request_digest = digest(8);
    assert!(matches!(
        repository_error(repository.begin_intent(conflicting))?,
        RepositoryError::IdempotencyConflict
    ));
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        connection.query_row("SELECT COUNT(run_id) FROM runs", [], |row| row
            .get::<_, u32>(0))?,
        1
    );
    assert_eq!(
        connection.query_row("SELECT COUNT(sequence) FROM events", [], |row| row
            .get::<_, u32>(0))?,
        1
    );
    Ok(())
}

#[test]
fn expired_lease_takeover_increments_fence_and_cell_revision_only_for_new_source() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("takeover.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let project_id = uuid_v7(100)?;
    let cell_id = uuid_v7(101)?;

    let first = repository.begin_intent(intent(10, project_id, cell_id, 1, 1, 20)?)?;
    assert_eq!(first.lease.fence.value(), 1);
    assert_eq!(first.run.cell_revision, 1);
    assert!(matches!(
        repository_error(repository.begin_intent(intent(11, project_id, cell_id, 1, 19, 30)?))?,
        RepositoryError::LeaseConflict
    ));

    let same_source = repository.begin_intent(intent(12, project_id, cell_id, 1, 20, 30)?)?;
    assert_eq!(same_source.lease.fence.value(), 2);
    assert_eq!(same_source.run.cell_revision, 1);
    let changed_source = repository.begin_intent(intent(13, project_id, cell_id, 2, 30, 40)?)?;
    assert_eq!(changed_source.lease.fence.value(), 3);
    assert_eq!(changed_source.run.cell_revision, 2);
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        connection.query_row(
            "SELECT last_fence FROM projects WHERE project_id = ?1",
            [project_id.to_string()],
            |row| row.get::<_, i64>(0),
        )?,
        3
    );
    assert_eq!(
        connection.query_row(
            "SELECT revision FROM cells WHERE cell_id = ?1",
            [cell_id.to_string()],
            |row| row.get::<_, i64>(0),
        )?,
        2
    );
    assert_eq!(
        connection.query_row("SELECT COUNT(run_id) FROM runs", [], |row| row
            .get::<_, u32>(0))?,
        3
    );
    Ok(())
}

#[test]
fn activate_and_start_commit_baseline_events_states_and_extended_lease() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("lifecycle.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let input = intent(20, uuid_v7(200)?, uuid_v7(201)?, 3, 1, 50)?;
    let begun = repository.begin_intent(input.clone())?;
    let baseline = checkpoint(20, 2);

    let active = repository.activate(
        begun.run.run_id,
        begun.lease,
        begun.run.source,
        &baseline,
        2,
    )?;
    assert_eq!(active.state, RunState::Pending);
    assert_eq!(active.cell_state, CellState::Queued);
    assert_eq!(active.transaction_state, TransactionState::Active);
    assert!(active.source_is_published);
    assert_eq!(active.baseline, Some(baseline.id));
    assert_eq!(active.last_sequence, 2);
    assert_eq!(active.revision, 3);

    assert!(matches!(
        repository_error(repository.start_execution(
            active.run_id,
            active.lease,
            digest(239),
            3,
            100,
        ))?,
        RepositoryError::WorkspaceBindingMismatch
    ));
    let running =
        repository.start_execution(active.run_id, active.lease, input.workspace_binding, 3, 100)?;
    assert_eq!(running.state, RunState::Running);
    assert_eq!(running.cell_state, CellState::Running);
    assert_eq!(running.transaction_state, TransactionState::Active);
    assert_eq!(running.lease.expires_at_ms, 100);
    assert_eq!(running.last_sequence, 3);
    assert_eq!(running.revision, 5);
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        connection.query_row(
            "SELECT expires_at_ms FROM project_leases WHERE project_id = ?1",
            [input.project_id.to_string()],
            |row| row.get::<_, i64>(0),
        )?,
        100
    );
    let kinds = {
        let mut statement =
            connection.prepare("SELECT kind FROM events WHERE run_id = ?1 ORDER BY sequence")?;
        statement
            .query_map([input.run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    assert_eq!(kinds, ["intent_created", "workspace_ready", "running"]);
    Ok(())
}

#[test]
fn another_projects_live_lease_cannot_activate_or_start_a_run() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("lease-binding.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let first_input = intent(25, uuid_v7(250)?, uuid_v7(251)?, 3, 1, 100)?;
    let second_input = intent(26, uuid_v7(260)?, uuid_v7(261)?, 4, 1, 100)?;
    let first = repository.begin_intent(first_input.clone())?;
    let second = repository.begin_intent(second_input)?;

    assert!(matches!(
        repository_error(repository.activate(
            first.run.run_id,
            second.lease,
            first.run.source,
            &checkpoint(25, 2),
            2,
        ))?,
        RepositoryError::FenceRejected
    ));
    let active = repository.activate(
        first.run.run_id,
        first.lease,
        first.run.source,
        &checkpoint(25, 2),
        2,
    )?;
    assert!(matches!(
        repository_error(repository.start_execution(
            active.run_id,
            second.lease,
            active.workspace_binding,
            3,
            200,
        ))?,
        RepositoryError::FenceRejected
    ));
    let unchanged = repository.run(active.run_id)?;
    assert_eq!(unchanged.state, RunState::Pending);
    assert_eq!(unchanged.transaction_state, TransactionState::Active);
    assert_eq!(repository.run(second.run.run_id)?, second.run);
    owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn expired_and_stale_fences_and_illegal_transitions_keep_typed_branches() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("fences.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let project_id = uuid_v7(300)?;
    let first_input = intent(30, project_id, uuid_v7(301)?, 4, 1, 10)?;
    let first = repository.begin_intent(first_input.clone())?;
    let first_active = repository.activate(
        first.run.run_id,
        first.lease,
        first.run.source,
        &checkpoint(30, 2),
        2,
    )?;

    assert!(matches!(
        repository_error(repository.start_execution(
            first_active.run_id,
            first_active.lease,
            first_active.workspace_binding,
            10,
            20,
        ))?,
        RepositoryError::FenceRejected
    ));
    let second_input = intent(31, project_id, uuid_v7(302)?, 5, 10, 30)?;
    let second = repository.begin_intent(second_input.clone())?;
    assert!(second.lease.fence > first.lease.fence);
    assert!(matches!(
        repository_error(repository.start_execution(
            first_active.run_id,
            first_active.lease,
            first_active.workspace_binding,
            11,
            20,
        ))?,
        RepositoryError::FenceRejected
    ));
    assert!(matches!(
        repository_error(repository.start_execution(
            uuid_v7(399)?,
            second.lease,
            second.run.workspace_binding,
            11,
            20,
        ))?,
        RepositoryError::NotFound
    ));
    assert!(matches!(
        repository_error(repository.start_execution(
            second.run.run_id,
            second.lease,
            second.run.workspace_binding,
            11,
            20,
        ))?,
        RepositoryError::InvalidTransition
    ));
    assert_eq!(replay(&repository, &first_input)?.state, RunState::Pending);
    assert_eq!(replay(&repository, &second_input)?.last_sequence, 1);
    owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn failed_activate_rolls_back_checkpoint_run_and_event_changes() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("rollback.db");
    let input = intent(40, uuid_v7(400)?, uuid_v7(401)?, 6, 1, 100)?;
    let begun = {
        let mut owner = open_owner(&database)?;
        let begun = owner.repository().begin_intent(input.clone())?;
        owner.shutdown(Duration::from_secs(2))?;
        begun
    };
    Connection::open(&database)?.execute(
        "UPDATE workspace_transactions SET state = 'active' WHERE transaction_id = ?1",
        [input.transaction_id.to_string()],
    )?;

    let baseline = checkpoint(40, 2);
    let mut owner = open_owner(&database)?;
    assert!(matches!(
        repository_error(owner.repository().activate(
            begun.run.run_id,
            begun.lease,
            begun.run.source,
            &baseline,
            2,
        ))?,
        RepositoryError::InvalidTransition
    ));
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(checkpoint_id) FROM checkpoints WHERE checkpoint_id = ?1",
            [baseline.id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        0
    );
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(path) FROM checkpoint_entries WHERE checkpoint_id = ?1",
            [baseline.id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        0
    );
    let run: (Option<String>, Option<String>, i64, i64) = connection.query_row(
        "SELECT source_object_digest, baseline_checkpoint_id, last_sequence, revision
         FROM runs WHERE run_id = ?1",
        [input.run_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(run, (None, None, 1, 1));
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(sequence) FROM events WHERE run_id = ?1",
            [input.run_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        1
    );
    Ok(())
}

#[test]
fn unsigned_values_outside_sqlite_range_fail_before_writes() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("overflow.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let mut overflow = intent(50, uuid_v7(500)?, uuid_v7(501)?, 7, 1, 100)?;
    overflow.source.length = u64::MAX;
    assert!(matches!(
        repository_error(repository.begin_intent(overflow))?,
        RepositoryError::NumericOverflow
    ));

    let input = intent(51, uuid_v7(502)?, uuid_v7(503)?, 8, 1, 100)?;
    let begun = repository.begin_intent(input)?;
    let mut baseline = checkpoint(50, 2);
    baseline.manifest.length = u64::MAX;
    assert!(matches!(
        repository_error(repository.activate(
            begun.run.run_id,
            begun.lease,
            begun.run.source,
            &baseline,
            2,
        ))?,
        RepositoryError::NumericOverflow
    ));
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        connection.query_row("SELECT COUNT(run_id) FROM runs", [], |row| row
            .get::<_, u32>(0))?,
        1
    );
    assert_eq!(
        connection.query_row("SELECT COUNT(checkpoint_id) FROM checkpoints", [], |row| {
            row.get::<_, u32>(0)
        })?,
        0
    );
    Ok(())
}
