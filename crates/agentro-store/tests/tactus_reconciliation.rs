use std::{error::Error, path::Path, str::FromStr, time::Duration};

use agentro_contracts::Sha256Digest;
use agentro_store::{
    JournalMode, StoreConfig,
    tactus::{
        model::{
            BeginIntent, BlobRef, CellState, CheckpointBackend, CheckpointKey, CheckpointRecord,
            FinishDisposition, FinishTerminal, RollbackFidelity, RunRecord, RunState,
            TransactionState,
        },
        repository::{MAX_RECONCILE_RUNS, Repository, RepositoryError, RepositoryOwner},
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

fn uuid_text(value: u64) -> String {
    format!("01890f3c-7b1d-7000-8000-{value:012x}")
}

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([byte; 32])
}

fn blob(byte: u8) -> BlobRef {
    BlobRef {
        digest: digest(byte),
        length: u64::from(byte),
    }
}

fn baseline(byte: u8) -> CheckpointRecord {
    CheckpointRecord {
        id: CheckpointKey::from_digest(digest(byte)),
        manifest: blob(byte),
        backend: CheckpointBackend::NonGit,
        fidelity: RollbackFidelity::FullManifest,
        git_context: None,
        entries: Vec::new(),
        total_file_bytes: 0,
        created_at_ms: 2,
    }
}

fn intent(seed: u64) -> Result<BeginIntent, Box<dyn Error>> {
    let byte = u8::try_from(seed % 200 + 1)?;
    Ok(BeginIntent {
        request_id: uuid_v7(seed * 10 + 1)?,
        request_digest: digest(byte),
        run_id: uuid_v7(seed * 10 + 2)?,
        transaction_id: uuid_v7(seed * 10 + 3)?,
        project_id: uuid_v7(seed * 10 + 4)?,
        cell_id: uuid_v7(seed * 10 + 5)?,
        source: blob(byte),
        workspace_binding: digest(240),
        owner_id: uuid_v7(seed * 10 + 6)?,
        now_ms: 1,
        expires_at_ms: 100,
    })
}

#[derive(Clone, Copy)]
enum IncompleteState {
    Pending,
    Running,
    Cancelling,
    Recovering,
}

struct Fixture {
    _temporary: TempDir,
    database: std::path::PathBuf,
    owner: RepositoryOwner,
    repository: Repository,
    run: RunRecord,
}

fn fixture(seed: u64, state: IncompleteState) -> Result<Fixture, Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("tactus.db");
    let owner = open_owner(&database)?;
    let repository = owner.repository();
    let input = intent(seed)?;
    let begun = repository.begin_intent(input.clone())?;
    let mut run = begun.run;
    if !matches!(state, IncompleteState::Pending) {
        let active = repository.activate(
            run.run_id,
            run.lease,
            run.source,
            &baseline(u8::try_from(seed % 200 + 20)?),
            2,
        )?;
        run = repository.start_execution(
            active.run_id,
            active.lease,
            input.workspace_binding,
            3,
            100,
        )?;
    }
    if matches!(state, IncompleteState::Cancelling) {
        run = repository.request_cancel(run.run_id, 4)?.0;
    }
    if matches!(state, IncompleteState::Recovering) {
        let mut connection = Connection::open(&database)?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE runs
             SET state = 'recovering', cell_state = 'recovering',
                 last_sequence = last_sequence + 1, revision = revision + 2,
                 updated_at_ms = 4
             WHERE run_id = ?1",
            [run.run_id.to_string()],
        )?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, kind, occurred_at_ms)
             VALUES (?1, 4, 'recovering', 4)",
            [run.run_id.to_string()],
        )?;
        transaction.commit()?;
        run = repository.run(run.run_id)?;
    }
    Ok(Fixture {
        _temporary: temporary,
        database,
        owner,
        repository,
        run,
    })
}

fn repository_error<T>(
    result: Result<T, RepositoryError>,
) -> Result<RepositoryError, Box<dyn Error>> {
    match result {
        Ok(_) => Err("repository operation unexpectedly succeeded".into()),
        Err(error) => Ok(error),
    }
}

fn lease_count(database: &Path, run: &RunRecord) -> Result<u32, Box<dyn Error>> {
    let fence = i64::try_from(run.lease.fence.value())?;
    Ok(Connection::open(database)?.query_row(
        "SELECT COUNT(project_id) FROM project_leases
         WHERE project_id = ?1 AND owner_id = ?2 AND fence = ?3",
        params![
            run.project_id.to_string(),
            run.lease.owner_id.to_string(),
            fence,
        ],
        |row| row.get(0),
    )?)
}

#[test]
fn every_incomplete_state_is_interrupted_once_in_stable_event_order() -> TestResult {
    for (seed, state) in [
        (10, IncompleteState::Pending),
        (20, IncompleteState::Running),
        (30, IncompleteState::Cancelling),
        (40, IncompleteState::Recovering),
    ] {
        let mut fixture = fixture(seed, state)?;
        let before = fixture.run.clone();
        assert_eq!(fixture.repository.reconcile_incomplete(10)?, 1);

        let interrupted = fixture.repository.run(before.run_id)?;
        assert_eq!(interrupted.state, RunState::Interrupted);
        assert_eq!(interrupted.cell_state, CellState::Interrupted);
        assert_eq!(interrupted.transaction_state, TransactionState::Abandoned);
        assert_eq!(interrupted.terminal_code.as_deref(), Some("PROCESS_DIED"));
        let kinds = fixture
            .repository
            .watch(before.run_id, 0, 100)?
            .into_iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>();
        assert!(kinds.ends_with(&["recovering".to_owned(), "interrupted".to_owned()]));
        assert_eq!(kinds.iter().filter(|kind| *kind == "recovering").count(), 1);
        assert_eq!(lease_count(&fixture.database, &before)?, 0);

        let event_count = kinds.len();
        assert_eq!(fixture.repository.reconcile_incomplete(11)?, 0);
        assert_eq!(
            fixture.repository.watch(before.run_id, 0, 100)?.len(),
            event_count
        );
        fixture.owner.shutdown(Duration::from_secs(2))?;
    }
    Ok(())
}

#[test]
fn terminal_runs_are_unchanged_and_nonmatching_leases_are_preserved() -> TestResult {
    let mut terminal = fixture(50, IncompleteState::Running)?;
    let finished = terminal.repository.finish_terminal(FinishTerminal {
        run_id: terminal.run.run_id,
        lease: terminal.run.lease,
        disposition: FinishDisposition::Failed,
        code: "WORKER_FAILED".to_owned(),
        environment: None,
        kernel_generation: None,
        now_ms: 4,
    })?;
    assert_eq!(terminal.repository.reconcile_incomplete(10)?, 0);
    assert_eq!(terminal.repository.run(finished.run_id)?, finished);
    terminal.owner.shutdown(Duration::from_secs(2))?;

    let mut mismatched = fixture(60, IncompleteState::Running)?;
    let replacement_owner = uuid_text(9_000);
    let replacement_fence = i64::try_from(mismatched.run.lease.fence.value())? + 1;
    Connection::open(&mismatched.database)?.execute(
        "UPDATE project_leases
         SET owner_id = ?2, fence = ?3, expires_at_ms = 500
         WHERE project_id = ?1",
        params![
            mismatched.run.project_id.to_string(),
            replacement_owner,
            replacement_fence,
        ],
    )?;

    assert_eq!(mismatched.repository.reconcile_incomplete(10)?, 1);
    assert_eq!(
        mismatched.repository.run(mismatched.run.run_id)?.state,
        RunState::Interrupted
    );
    let lease: (String, i64) = Connection::open(&mismatched.database)?.query_row(
        "SELECT owner_id, fence FROM project_leases WHERE project_id = ?1",
        [mismatched.run.project_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(lease, (replacement_owner, replacement_fence));
    mismatched.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn malformed_selected_identity_or_counter_fails_closed() -> TestResult {
    for (seed, corruption) in [
        (
            70,
            "UPDATE runs SET lease_owner_id = 'bad-id' WHERE state = 'running'",
        ),
        (80, "UPDATE runs SET revision = -1 WHERE state = 'running'"),
    ] {
        let mut fixture = fixture(seed, IncompleteState::Running)?;
        let connection = Connection::open(&fixture.database)?;
        connection.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
        connection.execute_batch(corruption)?;
        drop(connection);

        assert!(matches!(
            repository_error(fixture.repository.reconcile_incomplete(10))?,
            RepositoryError::CorruptState
        ));
        let row: (String, String, i64, i64) = Connection::open(&fixture.database)?.query_row(
            "SELECT state, cell_state, last_sequence, revision
             FROM runs WHERE run_id = ?1",
            [fixture.run.run_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(row.0, "running");
        assert_eq!(row.1, "running");
        assert_eq!(row.2, 3);
        assert_eq!(lease_count(&fixture.database, &fixture.run)?, 1);
        fixture.owner.shutdown(Duration::from_secs(2))?;
    }
    Ok(())
}

#[test]
fn event_failure_rolls_back_recovering_terminal_transaction_and_lease() -> TestResult {
    let mut fixture = fixture(90, IncompleteState::Running)?;
    Connection::open(&fixture.database)?.execute_batch(
        "CREATE TRIGGER reject_reconcile_interrupted
         BEFORE INSERT ON events
         WHEN NEW.kind = 'interrupted'
         BEGIN
             SELECT RAISE(ABORT, 'injected reconciliation failure');
         END;",
    )?;

    assert!(matches!(
        repository_error(fixture.repository.reconcile_incomplete(10))?,
        RepositoryError::Store(_)
    ));
    let unchanged = fixture.repository.run(fixture.run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!(unchanged.cell_state, CellState::Running);
    assert_eq!(unchanged.transaction_state, TransactionState::Active);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));
    assert_eq!(lease_count(&fixture.database, &fixture.run)?, 1);
    assert_eq!(
        fixture.repository.watch(fixture.run.run_id, 0, 100)?.len(),
        3
    );
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn reconciliation_refuses_more_than_the_current_scale_contract() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("bounded.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let mut connection = Connection::open(&database)?;
    let transaction = connection.transaction()?;
    let digest = digest(1).to_string();
    for index in 0..=MAX_RECONCILE_RUNS {
        let base = 20_000 + u64::from(index) * 6;
        let project_id = uuid_text(base);
        let cell_id = uuid_text(base + 1);
        let run_id = uuid_text(base + 2);
        let transaction_id = uuid_text(base + 3);
        let request_id = uuid_text(base + 4);
        let owner_id = uuid_text(base + 5);
        transaction.execute(
            "INSERT INTO projects (project_id, last_fence) VALUES (?1, 1)",
            [&project_id],
        )?;
        transaction.execute(
            "INSERT INTO cells
                 (cell_id, revision, source_digest, created_at_ms, updated_at_ms)
             VALUES (?1, 1, ?2, ?3, ?3)",
            params![cell_id, digest, index],
        )?;
        transaction.execute(
            "INSERT INTO runs (
                 run_id, request_id, request_digest, project_id, transaction_id,
                 cell_id, cell_revision, lease_owner_id, fence, lease_expires_at_ms,
                 workspace_binding_digest, state, cell_state, source_digest,
                 source_length, last_sequence, revision, created_at_ms, updated_at_ms
             ) VALUES (
                 ?1, ?2, ?7, ?3, ?4, ?5, 1, ?6, 1, 100,
                 ?7, 'pending', 'queued', ?7, 1, 1, 1, ?8, ?8
             )",
            params![
                run_id,
                request_id,
                project_id,
                transaction_id,
                cell_id,
                owner_id,
                digest,
                index,
            ],
        )?;
        transaction.execute(
            "INSERT INTO workspace_transactions
                 (transaction_id, run_id, project_id, fence, state,
                  created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 1, 'prepared', ?4, ?4)",
            params![transaction_id, run_id, project_id, index],
        )?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, kind, occurred_at_ms)
             VALUES (?1, 1, 'intent_created', ?2)",
            params![run_id, index],
        )?;
    }
    transaction.commit()?;

    assert!(matches!(
        repository_error(repository.reconcile_incomplete(10))?,
        RepositoryError::ReconciliationLimitExceeded { limit }
            if limit == MAX_RECONCILE_RUNS
    ));
    assert_eq!(
        Connection::open(&database)?.query_row(
            "SELECT COUNT(run_id) FROM runs WHERE state = 'recovering'",
            [],
            |row| row.get::<_, u32>(0),
        )?,
        0
    );
    owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}
