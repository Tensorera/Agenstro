use std::{error::Error, path::Path, str::FromStr, time::Duration};

use agentro_contracts::Sha256Digest;
use agentro_store::{
    JournalMode, StoreConfig,
    tactus::{
        model::{
            BeginIntent, BlobRef, CellKey, CellState, CheckpointBackend, CheckpointEntry,
            CheckpointEntryKind, CheckpointKey, CheckpointRecord, FinishSuccess, ProjectKey,
            RollbackFidelity, RunRecord, RunState, TransactionState,
        },
        repository::{Repository, RepositoryError, RepositoryOwner},
    },
};
use rusqlite::{Connection, params};
use tempfile::{TempDir, tempdir};

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

fn blob(byte: u8, length: u64) -> BlobRef {
    BlobRef {
        digest: digest(byte),
        length,
    }
}

fn intent(
    seed: u64,
    project_id: ProjectKey,
    cell_id: CellKey,
    now_ms: u64,
    expires_at_ms: u64,
) -> Result<BeginIntent, Box<dyn Error>> {
    Ok(BeginIntent {
        request_id: uuid_v7(seed * 10 + 1)?,
        request_digest: digest(u8::try_from(seed % 200 + 1)?),
        run_id: uuid_v7(seed * 10 + 2)?,
        transaction_id: uuid_v7(seed * 10 + 3)?,
        project_id,
        cell_id,
        source: blob(u8::try_from(seed % 200 + 1)?, 11),
        workspace_binding: digest(240),
        owner_id: uuid_v7(seed * 10 + 4)?,
        now_ms,
        expires_at_ms,
    })
}

fn empty_checkpoint(byte: u8, created_at_ms: u64) -> CheckpointRecord {
    CheckpointRecord {
        id: CheckpointKey::from_digest(digest(byte)),
        manifest: blob(byte, 13),
        backend: CheckpointBackend::NonGit,
        fidelity: RollbackFidelity::FullManifest,
        git_context: None,
        entries: Vec::new(),
        total_file_bytes: 0,
        created_at_ms,
    }
}

fn multi_checkpoint(byte: u8, created_at_ms: u64) -> CheckpointRecord {
    CheckpointRecord {
        id: CheckpointKey::from_digest(digest(byte)),
        manifest: blob(byte, 89),
        backend: CheckpointBackend::GitAware,
        fidelity: RollbackFidelity::DeclaredPaths,
        git_context: Some(digest(byte.wrapping_add(1))),
        entries: vec![
            CheckpointEntry {
                path: "zeta/run.sh".to_owned(),
                kind: CheckpointEntryKind::File,
                object: blob(byte.wrapping_add(2), 17),
                is_executable: true,
            },
            CheckpointEntry {
                path: "alpha/link".to_owned(),
                kind: CheckpointEntryKind::Symlink,
                object: blob(byte.wrapping_add(3), 6),
                is_executable: false,
            },
            CheckpointEntry {
                path: "middle/data.bin".to_owned(),
                kind: CheckpointEntryKind::File,
                object: blob(byte.wrapping_add(4), 23),
                is_executable: false,
            },
        ],
        total_file_bytes: 40,
        created_at_ms,
    }
}

fn sorted(mut checkpoint: CheckpointRecord) -> CheckpointRecord {
    checkpoint
        .entries
        .sort_by(|left, right| left.path.cmp(&right.path));
    checkpoint
}

fn repository_error<T>(
    result: Result<T, RepositoryError>,
) -> Result<RepositoryError, Box<dyn Error>> {
    match result {
        Ok(_) => Err("repository operation unexpectedly succeeded".into()),
        Err(error) => Ok(error),
    }
}

fn finish(run: &RunRecord, result: &CheckpointRecord, now_ms: u64) -> FinishSuccess {
    FinishSuccess {
        run_id: run.run_id,
        lease: run.lease,
        result: result.clone(),
        environment: digest(230),
        kernel_generation: 7,
        now_ms,
    }
}

struct RunningFixture {
    _temporary: TempDir,
    database: std::path::PathBuf,
    owner: RepositoryOwner,
    repository: Repository,
    run: RunRecord,
}

fn running_fixture(
    seed: u64,
    baseline: CheckpointRecord,
) -> Result<RunningFixture, Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("tactus.db");
    let owner = open_owner(&database)?;
    let repository = owner.repository();
    let input = intent(
        seed,
        uuid_v7(seed * 10 + 5)?,
        uuid_v7(seed * 10 + 6)?,
        1,
        100,
    )?;
    let begun = repository.begin_intent(input.clone())?;
    let active = repository.activate(
        begun.run.run_id,
        begun.lease,
        begun.run.source,
        &baseline,
        2,
    )?;
    let run =
        repository.start_execution(active.run_id, active.lease, input.workspace_binding, 3, 100)?;
    Ok(RunningFixture {
        _temporary: temporary,
        database,
        owner,
        repository,
        run,
    })
}

#[test]
fn checkpoint_lookup_round_trips_empty_and_returns_multiple_entries_sorted() -> TestResult {
    let baseline = empty_checkpoint(10, 2);
    let mut fixture = running_fixture(10, baseline.clone())?;
    assert_eq!(fixture.repository.checkpoint(baseline.id)?, baseline);

    let result = multi_checkpoint(20, 4);
    let succeeded = fixture
        .repository
        .finish_success(finish(&fixture.run, &result, 4))?;
    assert_eq!(succeeded.result, Some(result.id));
    assert_eq!(
        fixture.repository.checkpoint(result.id)?,
        sorted(result.clone())
    );
    assert!(matches!(
        repository_error(
            fixture
                .repository
                .checkpoint(CheckpointKey::from_digest(digest(99)))
        )?,
        RepositoryError::NotFound
    ));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn duplicate_checkpoint_registration_keeps_primary_key_content() -> TestResult {
    let baseline = multi_checkpoint(30, 2);
    let expected = sorted(baseline.clone());
    let mut fixture = running_fixture(20, baseline.clone())?;

    let succeeded = fixture
        .repository
        .finish_success(finish(&fixture.run, &baseline, 4))?;
    assert_eq!(succeeded.result, Some(baseline.id));
    assert_eq!(fixture.repository.checkpoint(baseline.id)?, expected);

    fixture.owner.shutdown(Duration::from_secs(2))?;
    let connection = Connection::open(&fixture.database)?;
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(checkpoint_id) FROM checkpoints WHERE checkpoint_id = ?1",
            [baseline.id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        1
    );
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(path) FROM checkpoint_entries WHERE checkpoint_id = ?1",
            [baseline.id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        3
    );
    Ok(())
}

#[test]
fn checkpoint_identity_collision_rejects_success_without_mutation() -> TestResult {
    let baseline = multi_checkpoint(35, 2);
    let mut fixture = running_fixture(25, baseline.clone())?;
    let mut conflicting = baseline;
    conflicting.manifest = blob(200, 91);

    assert!(matches!(
        repository_error(
            fixture
                .repository
                .finish_success(finish(&fixture.run, &conflicting, 4))
        )?,
        RepositoryError::CorruptState
    ));
    let unchanged = fixture.repository.run(fixture.run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!(unchanged.result, None);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn checkpoint_lookup_fails_closed_for_bad_count_kind_digest_and_length() -> TestResult {
    let mutations = [
        "UPDATE checkpoints SET entry_count = entry_count + 1 WHERE checkpoint_id = ?1",
        "UPDATE checkpoint_entries SET kind = 'directory' WHERE checkpoint_id = ?1 AND path = 'alpha/link'",
        "UPDATE checkpoints SET manifest_digest = 'bad-digest' WHERE checkpoint_id = ?1",
        "UPDATE checkpoint_entries SET object_digest = 'bad-digest' WHERE checkpoint_id = ?1 AND path = 'alpha/link'",
        "UPDATE checkpoints SET manifest_length = -1 WHERE checkpoint_id = ?1",
        "UPDATE checkpoint_entries SET object_length = -1 WHERE checkpoint_id = ?1 AND path = 'alpha/link'",
        "UPDATE checkpoints SET total_file_bytes = -1 WHERE checkpoint_id = ?1",
    ];

    for (index, mutation) in mutations.into_iter().enumerate() {
        let byte = u8::try_from(40 + index)?;
        let checkpoint = multi_checkpoint(byte, 2);
        let mut fixture = running_fixture(30 + u64::try_from(index)?, checkpoint.clone())?;
        let connection = Connection::open(&fixture.database)?;
        connection.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
        assert_eq!(
            connection.execute(mutation, [checkpoint.id.to_string()])?,
            1
        );
        drop(connection);

        assert!(matches!(
            repository_error(fixture.repository.checkpoint(checkpoint.id))?,
            RepositoryError::CorruptState
        ));
        fixture.owner.shutdown(Duration::from_secs(2))?;
    }
    Ok(())
}

#[test]
fn finish_success_commits_checkpoint_run_transaction_event_and_lease() -> TestResult {
    let mut fixture = running_fixture(50, empty_checkpoint(60, 2))?;
    let result = multi_checkpoint(61, 4);

    let succeeded = fixture
        .repository
        .finish_success(finish(&fixture.run, &result, 4))?;
    assert_eq!(succeeded.state, RunState::Succeeded);
    assert_eq!(succeeded.cell_state, CellState::Succeeded);
    assert_eq!(succeeded.transaction_state, TransactionState::Committed);
    assert_eq!(succeeded.result, Some(result.id));
    assert_eq!(succeeded.environment, Some(digest(230)));
    assert_eq!(succeeded.kernel_generation, Some(7));
    assert_eq!(succeeded.terminal_code, None);
    assert_eq!((succeeded.last_sequence, succeeded.revision), (4, 7));
    assert_eq!(
        fixture.repository.watch(succeeded.run_id, 3, 1)?[0].kind,
        "succeeded"
    );
    assert_eq!(
        fixture.repository.checkpoint(result.id)?,
        sorted(result.clone())
    );

    fixture.owner.shutdown(Duration::from_secs(2))?;
    let connection = Connection::open(&fixture.database)?;
    let transaction: (String, Option<String>) = connection.query_row(
        "SELECT state, result_checkpoint_id FROM workspace_transactions WHERE run_id = ?1",
        [succeeded.run_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(
        transaction,
        ("committed".to_owned(), Some(result.id.to_string()))
    );
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(project_id) FROM project_leases WHERE project_id = ?1",
            [succeeded.project_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        0
    );
    Ok(())
}

#[test]
fn stale_fence_rejects_success_without_partial_checkpoint_or_lease_release() -> TestResult {
    let mut fixture = running_fixture(60, empty_checkpoint(70, 2))?;
    let old_run = fixture.run.clone();
    let takeover = fixture.repository.begin_intent(intent(
        61,
        old_run.project_id,
        uuid_v7(699)?,
        old_run.lease.expires_at_ms,
        200,
    )?)?;
    let result = multi_checkpoint(71, 101);

    assert!(matches!(
        repository_error(
            fixture
                .repository
                .finish_success(finish(&old_run, &result, 101))
        )?,
        RepositoryError::FenceRejected
    ));
    let unchanged = fixture.repository.run(old_run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!(unchanged.cell_state, CellState::Running);
    assert_eq!(unchanged.transaction_state, TransactionState::Active);
    assert_eq!(unchanged.result, None);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));
    assert!(matches!(
        repository_error(fixture.repository.checkpoint(result.id))?,
        RepositoryError::NotFound
    ));

    let connection = Connection::open(&fixture.database)?;
    let lease: (String, i64) = connection.query_row(
        "SELECT owner_id, fence FROM project_leases WHERE project_id = ?1",
        [old_run.project_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(lease.0, takeover.lease.owner_id.to_string());
    assert_eq!(lease.1, i64::try_from(takeover.lease.fence.value())?);
    drop(connection);
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn corrupt_transaction_binding_rejects_success_before_checkpoint_registration() -> TestResult {
    let mut fixture = running_fixture(70, empty_checkpoint(80, 2))?;
    Connection::open(&fixture.database)?.execute(
        "UPDATE workspace_transactions SET fence = fence + 1 WHERE run_id = ?1",
        [fixture.run.run_id.to_string()],
    )?;
    let result = multi_checkpoint(81, 4);

    assert!(matches!(
        repository_error(
            fixture
                .repository
                .finish_success(finish(&fixture.run, &result, 4))
        )?,
        RepositoryError::CorruptState
    ));
    assert!(matches!(
        repository_error(fixture.repository.checkpoint(result.id))?,
        RepositoryError::NotFound
    ));
    assert!(matches!(
        repository_error(fixture.repository.run(fixture.run.run_id))?,
        RepositoryError::CorruptState
    ));
    let raw: (String, i64, i64, Option<String>) = Connection::open(&fixture.database)?.query_row(
        "SELECT state, last_sequence, revision, result_checkpoint_id
         FROM runs WHERE run_id = ?1",
        [fixture.run.run_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(raw, ("running".to_owned(), 3, 5, None));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn success_event_failure_rolls_back_checkpoint_states_counters_and_lease() -> TestResult {
    let mut fixture = running_fixture(80, empty_checkpoint(90, 2))?;
    Connection::open(&fixture.database)?.execute_batch(
        "CREATE TRIGGER reject_success_event
         BEFORE INSERT ON events
         WHEN NEW.kind = 'succeeded'
         BEGIN
             SELECT RAISE(ABORT, 'injected success event failure');
         END;",
    )?;
    let result = multi_checkpoint(91, 4);

    assert!(matches!(
        repository_error(
            fixture
                .repository
                .finish_success(finish(&fixture.run, &result, 4))
        )?,
        RepositoryError::Store(_)
    ));
    assert!(matches!(
        repository_error(fixture.repository.checkpoint(result.id))?,
        RepositoryError::NotFound
    ));
    let unchanged = fixture.repository.run(fixture.run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!(unchanged.cell_state, CellState::Running);
    assert_eq!(unchanged.transaction_state, TransactionState::Active);
    assert_eq!(unchanged.result, None);
    assert_eq!(unchanged.environment, None);
    assert_eq!(unchanged.kernel_generation, None);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));

    let connection = Connection::open(&fixture.database)?;
    let transaction: (String, Option<String>) = connection.query_row(
        "SELECT state, result_checkpoint_id FROM workspace_transactions WHERE run_id = ?1",
        [unchanged.run_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(transaction, ("active".to_owned(), None));
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(project_id) FROM project_leases
             WHERE project_id = ?1 AND owner_id = ?2 AND fence = ?3",
            params![
                unchanged.project_id.to_string(),
                unchanged.lease.owner_id.to_string(),
                i64::try_from(unchanged.lease.fence.value())?
            ],
            |row| row.get::<_, u32>(0),
        )?,
        1
    );
    drop(connection);
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn cancelling_success_closes_cancelled_without_registering_result() -> TestResult {
    let mut fixture = running_fixture(90, empty_checkpoint(100, 2))?;
    let (cancelling, should_signal) = fixture.repository.request_cancel(fixture.run.run_id, 4)?;
    assert!(should_signal);
    let result = multi_checkpoint(101, 5);

    let cancelled = fixture
        .repository
        .finish_success(finish(&cancelling, &result, 5))?;
    assert_eq!(cancelled.state, RunState::Cancelled);
    assert_eq!(cancelled.cell_state, CellState::Cancelled);
    assert_eq!(cancelled.transaction_state, TransactionState::Abandoned);
    assert_eq!(cancelled.terminal_code.as_deref(), Some("CANCELLED"));
    assert_eq!(cancelled.result, None);
    assert_eq!(cancelled.environment, None);
    assert_eq!(cancelled.kernel_generation, None);
    assert_eq!((cancelled.last_sequence, cancelled.revision), (5, 9));
    assert!(matches!(
        repository_error(fixture.repository.checkpoint(result.id))?,
        RepositoryError::NotFound
    ));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn success_numeric_overflow_fails_before_any_write() -> TestResult {
    let mut fixture = running_fixture(100, empty_checkpoint(110, 2))?;
    let mut result = multi_checkpoint(111, 4);
    result.entries[0].object.length = u64::MAX;

    assert!(matches!(
        repository_error(
            fixture
                .repository
                .finish_success(finish(&fixture.run, &result, 4))
        )?,
        RepositoryError::NumericOverflow
    ));
    let unchanged = fixture.repository.run(fixture.run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!(unchanged.result, None);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));
    assert!(matches!(
        repository_error(fixture.repository.checkpoint(result.id))?,
        RepositoryError::NotFound
    ));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}
