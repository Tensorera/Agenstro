use std::{error::Error, path::Path, str::FromStr, time::Duration};

use agentro_contracts::Sha256Digest;
use agentro_store::{
    JournalMode, StoreConfig,
    tactus::{
        model::{
            BeginIntent, BlobRef, CellKey, CellState, CheckpointBackend, CheckpointKey,
            CheckpointRecord, FinishDisposition, FinishTerminal, LeaseGrant, ProjectKey,
            RollbackFidelity, RunKey, RunRecord, RunState, TransactionState,
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

fn blob(byte: u8) -> BlobRef {
    BlobRef {
        digest: digest(byte),
        length: u64::from(byte),
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
        source: blob(u8::try_from(seed % 200 + 1)?),
        workspace_binding: digest(240),
        owner_id: uuid_v7(seed * 10 + 4)?,
        now_ms,
        expires_at_ms,
    })
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

fn repository_error<T>(
    result: Result<T, RepositoryError>,
) -> Result<RepositoryError, Box<dyn Error>> {
    match result {
        Ok(_) => Err("repository operation unexpectedly succeeded".into()),
        Err(error) => Ok(error),
    }
}

fn finish(
    run_id: RunKey,
    lease: LeaseGrant,
    disposition: FinishDisposition,
    code: &str,
    now_ms: u64,
) -> FinishTerminal {
    FinishTerminal {
        run_id,
        lease,
        disposition,
        code: code.to_owned(),
        environment: Some(digest(230)),
        kernel_generation: Some(7),
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

fn running_fixture(seed: u64) -> Result<RunningFixture, Box<dyn Error>> {
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
        &baseline(u8::try_from(seed % 200 + 20)?),
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
fn pending_cancel_is_atomic_and_terminal_cancel_is_rejected() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("pending-cancel.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let input = intent(10, uuid_v7(1)?, uuid_v7(2)?, 1, 100)?;
    let begun = repository.begin_intent(input)?;

    let (cancelled, should_signal) = repository.request_cancel(begun.run.run_id, 2)?;
    assert!(!should_signal);
    assert_eq!(cancelled.state, RunState::Cancelled);
    assert_eq!(cancelled.cell_state, CellState::Cancelled);
    assert_eq!(cancelled.transaction_state, TransactionState::Abandoned);
    assert_eq!(cancelled.terminal_code.as_deref(), Some("CANCELLED"));
    assert_eq!((cancelled.last_sequence, cancelled.revision), (2, 3));
    assert!(matches!(
        repository_error(repository.request_cancel(cancelled.run_id, 3))?,
        RepositoryError::InvalidTransition
    ));
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(project_id) FROM project_leases WHERE project_id = ?1",
            [cancelled.project_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        0
    );
    let kinds = {
        let mut statement =
            connection.prepare("SELECT kind FROM events WHERE run_id = ?1 ORDER BY sequence")?;
        statement
            .query_map([cancelled.run_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    assert_eq!(kinds, ["intent_created", "cancelled"]);
    Ok(())
}

#[test]
fn running_cancel_replays_without_counters_and_forces_cancelled_finish() -> TestResult {
    let mut fixture = running_fixture(20)?;
    let (cancelling, should_signal) = fixture.repository.request_cancel(fixture.run.run_id, 4)?;
    assert!(should_signal);
    assert_eq!(cancelling.state, RunState::Cancelling);
    assert_eq!(cancelling.cell_state, CellState::Running);
    assert_eq!(cancelling.transaction_state, TransactionState::Active);
    assert_eq!((cancelling.last_sequence, cancelling.revision), (4, 7));

    let (replayed, should_signal) = fixture.repository.request_cancel(cancelling.run_id, 5)?;
    assert!(should_signal);
    assert_eq!(replayed, cancelling);

    let cancelled = fixture.repository.finish_terminal(finish(
        cancelling.run_id,
        cancelling.lease,
        FinishDisposition::Failed,
        "CANCELLED",
        6,
    ))?;
    assert_eq!(cancelled.state, RunState::Cancelled);
    assert_eq!(cancelled.cell_state, CellState::Cancelled);
    assert_eq!(cancelled.transaction_state, TransactionState::Abandoned);
    assert_eq!(cancelled.terminal_code.as_deref(), Some("CANCELLED"));
    assert_eq!((cancelled.last_sequence, cancelled.revision), (5, 9));
    fixture.owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(&fixture.database)?;
    let kinds = {
        let mut statement =
            connection.prepare("SELECT kind FROM events WHERE run_id = ?1 ORDER BY sequence")?;
        statement
            .query_map([cancelled.run_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    assert_eq!(
        kinds,
        [
            "intent_created",
            "workspace_ready",
            "running",
            "cancel_requested",
            "cancelled"
        ]
    );
    Ok(())
}

#[test]
fn every_closed_disposition_persists_terminal_state_code_and_metadata() -> TestResult {
    for (seed, disposition, code, run_state, cell_state, transaction_state, event_kind) in [
        (
            30,
            FinishDisposition::Failed,
            "WORKER_FAILED",
            RunState::Failed,
            CellState::Failed,
            TransactionState::Abandoned,
            "failed",
        ),
        (
            31,
            FinishDisposition::Cancelled,
            "CANCELLED",
            RunState::Cancelled,
            CellState::Cancelled,
            TransactionState::Abandoned,
            "cancelled",
        ),
        (
            32,
            FinishDisposition::Interrupted,
            "PROCESS_DIED",
            RunState::Interrupted,
            CellState::Interrupted,
            TransactionState::Abandoned,
            "interrupted",
        ),
        (
            33,
            FinishDisposition::Conflict,
            "WORKSPACE_CHANGED",
            RunState::Failed,
            CellState::Failed,
            TransactionState::Conflict,
            "workspace_conflict",
        ),
    ] {
        let mut fixture = running_fixture(seed)?;
        let terminal = fixture.repository.finish_terminal(finish(
            fixture.run.run_id,
            fixture.run.lease,
            disposition,
            code,
            4,
        ))?;
        assert_eq!(terminal.state, run_state);
        assert_eq!(terminal.cell_state, cell_state);
        assert_eq!(terminal.transaction_state, transaction_state);
        assert_eq!(terminal.terminal_code.as_deref(), Some(code));
        assert_eq!(terminal.environment, Some(digest(230)));
        assert_eq!(terminal.kernel_generation, Some(7));
        assert_eq!((terminal.last_sequence, terminal.revision), (4, 7));
        assert_eq!(
            fixture.repository.watch(terminal.run_id, 3, 1)?[0].kind,
            event_kind
        );
        fixture.owner.shutdown(Duration::from_secs(2))?;

        let connection = Connection::open(&fixture.database)?;
        assert_eq!(
            connection.query_row(
                "SELECT COUNT(project_id) FROM project_leases WHERE project_id = ?1",
                [terminal.project_id.to_string()],
                |row| row.get::<_, u32>(0),
            )?,
            0
        );
    }
    Ok(())
}

#[test]
fn stale_run_fence_cannot_finish_or_release_takeover_lease() -> TestResult {
    let mut fixture = running_fixture(40)?;
    let old_run = fixture.run.clone();
    assert!(matches!(
        repository_error(fixture.repository.finish_terminal(finish(
            old_run.run_id,
            old_run.lease,
            FinishDisposition::Interrupted,
            "EXPIRED_WORKER",
            old_run.lease.expires_at_ms,
        )))?,
        RepositoryError::FenceRejected
    ));
    assert!(matches!(
        repository_error(
            fixture
                .repository
                .request_cancel(old_run.run_id, old_run.lease.expires_at_ms)
        )?,
        RepositoryError::FenceRejected
    ));
    let takeover_input = intent(
        41,
        old_run.project_id,
        uuid_v7(419)?,
        old_run.lease.expires_at_ms,
        200,
    )?;
    let takeover = fixture.repository.begin_intent(takeover_input)?;
    assert!(takeover.lease.fence > old_run.lease.fence);

    assert!(matches!(
        repository_error(fixture.repository.finish_terminal(finish(
            old_run.run_id,
            old_run.lease,
            FinishDisposition::Interrupted,
            "STALE_WORKER",
            101,
        )))?,
        RepositoryError::FenceRejected
    ));
    assert!(matches!(
        repository_error(fixture.repository.request_cancel(old_run.run_id, 101))?,
        RepositoryError::FenceRejected
    ));
    let unchanged = fixture.repository.run(old_run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));

    let connection = Connection::open(&fixture.database)?;
    let current: (String, i64) = connection.query_row(
        "SELECT owner_id, fence FROM project_leases WHERE project_id = ?1",
        [old_run.project_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(current.0, takeover.lease.owner_id.to_string());
    assert_eq!(current.1, i64::try_from(takeover.lease.fence.value())?);
    drop(connection);
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn recovering_run_rejects_cancel_without_mutation() -> TestResult {
    let mut fixture = running_fixture(50)?;
    Connection::open(&fixture.database)?.execute(
        "UPDATE runs SET state = 'recovering', cell_state = 'recovering' WHERE run_id = ?1",
        [fixture.run.run_id.to_string()],
    )?;

    assert!(matches!(
        repository_error(fixture.repository.request_cancel(fixture.run.run_id, 4))?,
        RepositoryError::InvalidTransition
    ));
    let recovering = fixture.repository.run(fixture.run.run_id)?;
    assert_eq!(recovering.state, RunState::Recovering);
    assert_eq!(recovering.cell_state, CellState::Recovering);
    assert_eq!((recovering.last_sequence, recovering.revision), (3, 5));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn terminal_event_failure_rolls_back_run_transaction_and_lease() -> TestResult {
    let mut fixture = running_fixture(60)?;
    Connection::open(&fixture.database)?.execute_batch(
        "CREATE TRIGGER reject_terminal_event
         BEFORE INSERT ON events
         WHEN NEW.kind IN ('failed', 'cancelled', 'interrupted', 'workspace_conflict')
         BEGIN
             SELECT RAISE(ABORT, 'injected terminal event failure');
         END;",
    )?;

    assert!(matches!(
        repository_error(fixture.repository.finish_terminal(finish(
            fixture.run.run_id,
            fixture.run.lease,
            FinishDisposition::Failed,
            "WORKER_FAILED",
            4,
        )))?,
        RepositoryError::Store(_)
    ));
    let unchanged = fixture.repository.run(fixture.run.run_id)?;
    assert_eq!(unchanged.state, RunState::Running);
    assert_eq!(unchanged.cell_state, CellState::Running);
    assert_eq!(unchanged.transaction_state, TransactionState::Active);
    assert_eq!(unchanged.terminal_code, None);
    assert_eq!(unchanged.environment, None);
    assert_eq!(unchanged.kernel_generation, None);
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));

    let connection = Connection::open(&fixture.database)?;
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
