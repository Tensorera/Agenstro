use std::{error::Error, path::Path, str::FromStr, time::Duration};

use agentro_contracts::Sha256Digest;
use agentro_store::{
    JournalMode, StoreConfig,
    tactus::{
        model::{
            AppendOutput, BeginIntent, BlobRef, CheckpointBackend, CheckpointKey, CheckpointRecord,
            LeaseGrant, OutputBudget, OutputStream, RollbackFidelity, RunKey,
        },
        repository::{MAX_WATCH_EVENTS, Repository, RepositoryError, RepositoryOwner},
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

fn output(
    run_id: RunKey,
    lease: LeaseGrant,
    worker_sequence: u64,
    stream: OutputStream,
    blob: BlobRef,
    now_ms: u64,
    budget: OutputBudget,
) -> AppendOutput {
    AppendOutput {
        run_id,
        lease,
        worker_sequence,
        stream,
        blob,
        now_ms,
        budget,
    }
}

fn intent(seed: u64) -> Result<BeginIntent, Box<dyn Error>> {
    Ok(BeginIntent {
        request_id: uuid_v7(seed * 10 + 1)?,
        request_digest: digest(1),
        run_id: uuid_v7(seed * 10 + 2)?,
        transaction_id: uuid_v7(seed * 10 + 3)?,
        project_id: uuid_v7(seed * 10 + 4)?,
        cell_id: uuid_v7(seed * 10 + 5)?,
        source: blob(1, 1),
        workspace_binding: digest(2),
        owner_id: uuid_v7(seed * 10 + 6)?,
        now_ms: 1,
        expires_at_ms: 1_000,
    })
}

fn baseline(byte: u8) -> CheckpointRecord {
    CheckpointRecord {
        id: CheckpointKey::from_digest(digest(byte)),
        manifest: blob(byte, 1),
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

struct RunningFixture {
    _temporary: TempDir,
    database: std::path::PathBuf,
    owner: RepositoryOwner,
    repository: Repository,
    run_id: RunKey,
    lease: LeaseGrant,
}

fn running_fixture(seed: u64) -> Result<RunningFixture, Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("tactus.db");
    let owner = open_owner(&database)?;
    let repository = owner.repository();
    let input = intent(seed)?;
    let begun = repository.begin_intent(input.clone())?;
    let active = repository.activate(
        begun.run.run_id,
        begun.lease,
        begun.run.source,
        &baseline(10),
        2,
    )?;
    let running = repository.start_execution(
        active.run_id,
        active.lease,
        input.workspace_binding,
        3,
        1_000,
    )?;
    Ok(RunningFixture {
        _temporary: temporary,
        database,
        owner,
        repository,
        run_id: running.run_id,
        lease: running.lease,
    })
}

#[test]
fn append_output_replays_same_worker_sequence_and_rejects_changed_payload() -> TestResult {
    let mut fixture = running_fixture(100)?;
    let budget = OutputBudget::new(10, 2)?;
    let first = fixture.repository.append_output(output(
        fixture.run_id,
        fixture.lease,
        1,
        OutputStream::Stdout,
        blob(20, 4),
        4,
        budget,
    ))?;
    assert_eq!(first, 4);
    assert_eq!(
        fixture.repository.append_output(output(
            fixture.run_id,
            fixture.lease,
            1,
            OutputStream::Stdout,
            blob(20, 4),
            5,
            OutputBudget::new(1, 1)?,
        ))?,
        first
    );
    assert!(matches!(
        repository_error(fixture.repository.append_output(output(
            fixture.run_id,
            fixture.lease,
            1,
            OutputStream::Stderr,
            blob(21, 4),
            5,
            budget,
        )))?,
        RepositoryError::InvalidTransition
    ));
    let run = fixture.repository.run(fixture.run_id)?;
    assert_eq!(run.last_sequence, 4);
    assert_eq!(run.revision, 6);

    fixture.owner.shutdown(Duration::from_secs(2))?;
    let connection = Connection::open(&fixture.database)?;
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(sequence) FROM events WHERE run_id = ?1 AND kind = 'output'",
            [fixture.run_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        1
    );
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(event_sequence) FROM output_chunks WHERE run_id = ?1",
            [fixture.run_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        1
    );
    Ok(())
}

#[test]
fn append_output_budget_and_stale_fence_leave_no_partial_event() -> TestResult {
    let mut fixture = running_fixture(200)?;
    let sequence = fixture.repository.append_output(output(
        fixture.run_id,
        fixture.lease,
        1,
        OutputStream::Display,
        blob(30, 4),
        4,
        OutputBudget::new(4, 1)?,
    ))?;
    assert_eq!(sequence, 4);
    for budget in [OutputBudget::new(4, 2)?, OutputBudget::new(10, 1)?] {
        assert!(matches!(
            repository_error(fixture.repository.append_output(output(
                fixture.run_id,
                fixture.lease,
                2,
                OutputStream::Stdout,
                blob(31, 1),
                5,
                budget,
            )))?,
            RepositoryError::OutputBudgetExceeded
        ));
    }
    let stale = LeaseGrant {
        fence: agentro_store::tactus::model::FencingToken::new(fixture.lease.fence.value() + 1)?,
        ..fixture.lease
    };
    assert!(matches!(
        repository_error(fixture.repository.append_output(output(
            fixture.run_id,
            stale,
            2,
            OutputStream::Stdout,
            blob(31, 1),
            5,
            OutputBudget::new(10, 2)?,
        )))?,
        RepositoryError::FenceRejected
    ));
    assert!(matches!(
        repository_error(fixture.repository.append_output(output(
            fixture.run_id,
            fixture.lease,
            2,
            OutputStream::Stdout,
            blob(31, 1),
            fixture.lease.expires_at_ms,
            OutputBudget::new(10, 2)?,
        )))?,
        RepositoryError::FenceRejected
    ));
    let run = fixture.repository.run(fixture.run_id)?;
    assert_eq!((run.last_sequence, run.revision), (4, 6));

    fixture.owner.shutdown(Duration::from_secs(2))?;
    let connection = Connection::open(&fixture.database)?;
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(sequence) FROM events WHERE run_id = ?1",
            [fixture.run_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        4
    );
    assert_eq!(
        connection.query_row(
            "SELECT COUNT(event_sequence) FROM output_chunks WHERE run_id = ?1",
            [fixture.run_id.to_string()],
            |row| row.get::<_, u32>(0),
        )?,
        1
    );
    Ok(())
}

#[test]
fn append_output_rejects_a_pending_run_without_mutation() -> TestResult {
    let temporary = tempdir()?;
    let database = temporary.path().join("pending.db");
    let mut owner = open_owner(&database)?;
    let repository = owner.repository();
    let begun = repository.begin_intent(intent(250)?)?;
    assert!(matches!(
        repository_error(repository.append_output(output(
            begun.run.run_id,
            begun.lease,
            1,
            OutputStream::Stdout,
            blob(35, 1),
            2,
            OutputBudget::new(10, 2)?,
        )))?,
        RepositoryError::InvalidTransition
    ));
    assert_eq!(repository.run(begun.run.run_id)?.last_sequence, 1);
    owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn another_projects_live_lease_cannot_append_output() -> TestResult {
    let mut fixture = running_fixture(275)?;
    let second_input = intent(276)?;
    let second = fixture.repository.begin_intent(second_input)?;

    assert!(matches!(
        repository_error(fixture.repository.append_output(output(
            fixture.run_id,
            second.lease,
            1,
            OutputStream::Stdout,
            blob(36, 1),
            4,
            OutputBudget::new(10, 10)?,
        )))?,
        RepositoryError::FenceRejected
    ));
    let unchanged = fixture.repository.run(fixture.run_id)?;
    assert_eq!((unchanged.last_sequence, unchanged.revision), (3, 5));
    assert_eq!(fixture.repository.run(second.run.run_id)?, second.run);
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn run_and_watch_return_stable_bounded_cursor_pages() -> TestResult {
    let mut fixture = running_fixture(300)?;
    for worker_sequence in 1..=3 {
        fixture.repository.append_output(output(
            fixture.run_id,
            fixture.lease,
            worker_sequence,
            OutputStream::Stdout,
            blob(40 + u8::try_from(worker_sequence)?, 1),
            3 + worker_sequence,
            OutputBudget::new(10, 10)?,
        ))?;
    }
    assert_eq!(fixture.repository.run(fixture.run_id)?.last_sequence, 6);
    assert!(matches!(
        repository_error(fixture.repository.run(uuid_v7(9_999)?))?,
        RepositoryError::NotFound
    ));

    let first = fixture.repository.watch(fixture.run_id, 0, 2)?;
    let second = fixture.repository.watch(fixture.run_id, 2, 2)?;
    let third = fixture.repository.watch(fixture.run_id, 4, 2)?;
    assert_eq!(
        first.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        second
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(
        third.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        [5, 6]
    );
    assert!(matches!(
        repository_error(fixture.repository.watch(fixture.run_id, 0, 0))?,
        RepositoryError::InvalidTransition
    ));
    assert!(matches!(
        repository_error(
            fixture
                .repository
                .watch(fixture.run_id, 0, MAX_WATCH_EVENTS + 1,)
        )?,
        RepositoryError::InvalidTransition
    ));
    assert!(matches!(
        repository_error(fixture.repository.watch(fixture.run_id, u64::MAX, 1))?,
        RepositoryError::NumericOverflow
    ));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

fn assert_corrupt_run(seed: u64, mutation: &str) -> TestResult {
    let mut fixture = running_fixture(seed)?;
    let connection = Connection::open(&fixture.database)?;
    connection.pragma_update(None, "ignore_check_constraints", true)?;
    connection.execute(mutation, [fixture.run_id.to_string()])?;
    drop(connection);

    assert!(matches!(
        repository_error(fixture.repository.run(fixture.run_id))?,
        RepositoryError::CorruptState
    ));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

fn assert_corrupt_event(seed: u64, mutation: &str) -> TestResult {
    let mut fixture = running_fixture(seed)?;
    fixture.repository.append_output(output(
        fixture.run_id,
        fixture.lease,
        1,
        OutputStream::Stdout,
        blob(50, 1),
        4,
        OutputBudget::new(10, 10)?,
    ))?;
    let connection = Connection::open(&fixture.database)?;
    connection.pragma_update(None, "ignore_check_constraints", true)?;
    connection.execute(mutation, params![fixture.run_id.to_string(), 4_i64])?;
    drop(connection);

    assert!(matches!(
        repository_error(fixture.repository.watch(fixture.run_id, 3, 1))?,
        RepositoryError::CorruptState
    ));
    fixture.owner.shutdown(Duration::from_secs(2))?;
    Ok(())
}

#[test]
fn run_row_mapper_fails_closed_for_states_ids_digests_and_counters() -> TestResult {
    for (seed, mutation) in [
        (400, "UPDATE runs SET state = 'unknown' WHERE run_id = ?1"),
        (
            401,
            "UPDATE workspace_transactions SET state = 'unknown' WHERE run_id = ?1",
        ),
        (
            402,
            "UPDATE runs SET lease_owner_id = 'bad-id' WHERE run_id = ?1",
        ),
        (
            403,
            "UPDATE runs SET source_digest = 'bad-digest' WHERE run_id = ?1",
        ),
        (404, "UPDATE runs SET last_sequence = 0 WHERE run_id = ?1"),
        (405, "UPDATE runs SET revision = 0 WHERE run_id = ?1"),
        (406, "UPDATE runs SET created_at_ms = -1 WHERE run_id = ?1"),
        (407, "UPDATE runs SET last_sequence = -1 WHERE run_id = ?1"),
        (408, "UPDATE runs SET revision = -1 WHERE run_id = ?1"),
        (
            409,
            "UPDATE workspace_transactions SET fence = fence + 1 WHERE run_id = ?1",
        ),
        (410, "UPDATE runs SET state = 'succeeded' WHERE run_id = ?1"),
        (
            411,
            "UPDATE workspace_transactions SET transaction_id = '01890f3c-7b1d-7000-8000-00000000ffff' WHERE run_id = ?1",
        ),
    ] {
        assert_corrupt_run(seed, mutation)?;
    }
    Ok(())
}

#[test]
fn event_row_mapper_fails_closed_for_stream_digest_and_sequences() -> TestResult {
    for (seed, mutation) in [
        (
            500,
            "UPDATE events SET stream = 'unknown' WHERE run_id = ?1 AND sequence = ?2",
        ),
        (
            501,
            "UPDATE events SET blob_digest = 'bad-digest' WHERE run_id = ?1 AND sequence = ?2",
        ),
        (
            502,
            "UPDATE events SET worker_sequence = 0 WHERE run_id = ?1 AND sequence = ?2",
        ),
        (
            503,
            "UPDATE events SET occurred_at_ms = -1 WHERE run_id = ?1 AND sequence = ?2",
        ),
        (
            504,
            "UPDATE events SET blob_length = NULL WHERE run_id = ?1 AND sequence = ?2",
        ),
        (
            505,
            "UPDATE events SET worker_sequence = -1 WHERE run_id = ?1 AND sequence = ?2",
        ),
        (
            506,
            "DELETE FROM output_chunks WHERE run_id = ?1 AND event_sequence = ?2",
        ),
    ] {
        assert_corrupt_event(seed, mutation)?;
    }
    Ok(())
}

#[test]
fn output_and_query_plans_use_run_scoped_indexes() -> TestResult {
    let mut fixture = running_fixture(600)?;
    fixture.owner.shutdown(Duration::from_secs(2))?;
    let connection = Connection::open(&fixture.database)?;
    let mut statement = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT e.sequence, e.kind, e.worker_sequence, e.stream,
                e.blob_digest, e.blob_length, e.occurred_at_ms,
                o.stream, o.blob_digest, o.blob_length
         FROM events e
         LEFT JOIN output_chunks o
           ON o.run_id = e.run_id AND o.event_sequence = e.sequence
         WHERE e.run_id = ?1 AND e.sequence > ?2
         ORDER BY e.sequence ASC
         LIMIT ?3",
    )?;
    let details = statement
        .query_map(params![fixture.run_id.to_string(), 0_i64, 10_i64], |row| {
            row.get::<_, String>(3)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    assert!(
        details.iter().any(|detail| {
            detail.contains("events_run_sequence_idx")
                || detail.contains("sqlite_autoindex_events_1")
        }),
        "watch plan did not use the run/sequence index: {details:?}"
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("sqlite_autoindex_output_chunks_1")),
        "watch plan did not use the output chunk primary key: {details:?}"
    );

    let mut statement = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT e.sequence, e.kind, e.stream, e.blob_digest, e.blob_length,
                o.stream, o.blob_digest, o.blob_length
         FROM events e
         LEFT JOIN output_chunks o
           ON o.run_id = e.run_id AND o.event_sequence = e.sequence
         WHERE e.run_id = ?1 AND e.worker_sequence = ?2",
    )?;
    let details = statement
        .query_map(params![fixture.run_id.to_string(), 1_i64], |row| {
            row.get::<_, String>(3)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("sqlite_autoindex_events_2")),
        "replay plan did not use the worker-sequence unique index: {details:?}"
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("sqlite_autoindex_output_chunks_1")),
        "replay plan did not use the output chunk primary key: {details:?}"
    );

    let mut statement = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT COUNT(event_sequence), COALESCE(SUM(blob_length), 0)
         FROM output_chunks WHERE run_id = ?1",
    )?;
    let details = statement
        .query_map([fixture.run_id.to_string()], |row| row.get::<_, String>(3))?
        .collect::<Result<Vec<_>, _>>()?;
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("sqlite_autoindex_output_chunks_1")),
        "budget plan did not use the output chunk primary key: {details:?}"
    );
    Ok(())
}
