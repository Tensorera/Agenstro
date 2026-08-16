use std::{error::Error, fmt::Write as _, path::Path, time::Duration};

use agentro_store::{
    JournalMode, StoreConfig,
    tactus::{
        codec::{
            CorruptStorageError, decode_boolean, decode_digest, decode_non_negative,
            decode_optional_blob, decode_positive, decode_project_key,
        },
        migration::{TACTUS_V1_MIGRATION_NAME, TACTUS_V1_SCHEMA_SQL},
        model::{
            CellState, CheckpointBackend, CheckpointEntryKind, OutputStream, RollbackFidelity,
            RunState, TransactionState,
        },
        repository::{MAX_WATCH_EVENTS, RepositoryOwner},
    },
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const LEGACY_TACTUS_V1_SCHEMA_SQL: &str = include_str!("fixtures/tactus_v1_schema.sql");
const EXPECTED_TACTUS_V1_CHECKSUM: &str =
    "66b61962506fb30b537d8c6ee3aeae99618f77b235764576b0ac1af113e93fcc";

fn config() -> Result<StoreConfig, agentro_store::StoreError> {
    StoreConfig::new(16, Duration::from_millis(250), JournalMode::Wal)
}

fn legacy_checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tactus.sqlite.migration.v1\0");
    hasher.update(1_u32.to_be_bytes());
    hasher.update(TACTUS_V1_MIGRATION_NAME.as_bytes());
    hasher.update([0]);
    hasher.update(sql.as_bytes());
    let mut encoded = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn schema_names(connection: &Connection, kind: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = ?1 AND name NOT LIKE 'sqlite_%' AND sql IS NOT NULL
         ORDER BY name",
    )?;
    statement
        .query_map([kind], |row| row.get::<_, String>(0))?
        .collect()
}

fn create_legacy_fixture(database: &Path, checksum: &str) -> Result<String, Box<dyn Error>> {
    let connection = Connection::open(database)?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE schema_migrations (
             version INTEGER PRIMARY KEY CHECK (version > 0),
             name TEXT NOT NULL UNIQUE,
             checksum TEXT NOT NULL
         );",
    )?;
    connection.execute_batch(LEGACY_TACTUS_V1_SCHEMA_SQL)?;
    connection.execute(
        "INSERT INTO schema_migrations (version, name, checksum) VALUES (?1, ?2, ?3)",
        params![1, TACTUS_V1_MIGRATION_NAME, checksum],
    )?;
    connection.execute(
        "INSERT INTO projects (project_id, last_fence) VALUES (?1, ?2)",
        params!["legacy-sentinel", 7],
    )?;
    connection
        .query_row(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

#[test]
fn tactus_schema_bytes_and_legacy_checksum_are_frozen() {
    assert_eq!(TACTUS_V1_MIGRATION_NAME, "create_tactus_runtime");
    assert_eq!(
        TACTUS_V1_SCHEMA_SQL.as_bytes(),
        LEGACY_TACTUS_V1_SCHEMA_SQL.as_bytes()
    );
    assert_eq!(
        legacy_checksum(TACTUS_V1_SCHEMA_SQL),
        EXPECTED_TACTUS_V1_CHECKSUM
    );
}

#[test]
fn new_database_has_tactus_v1_schema_constraints_and_indexes() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("new-tactus.db");
    let mut owner = RepositoryOwner::open(database.clone(), config()?, Duration::from_secs(2))?;
    owner
        .repository()
        .ensure_schema_ready(Duration::from_secs(1))?;
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    connection.execute_batch("PRAGMA foreign_keys = ON")?;
    assert_eq!(
        schema_names(&connection, "table")?,
        [
            "cells",
            "checkpoint_entries",
            "checkpoints",
            "events",
            "output_chunks",
            "project_leases",
            "projects",
            "runs",
            "schema_migrations",
            "workspace_transactions",
        ]
    );
    assert_eq!(
        schema_names(&connection, "index")?,
        [
            "checkpoint_entries_checkpoint_path_idx",
            "events_run_sequence_idx",
            "runs_project_created_idx",
        ]
    );
    assert!(
        connection
            .execute(
                "INSERT INTO projects (project_id, last_fence) VALUES (?1, ?2)",
                params!["bad-fence", -1],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO project_leases
                 (project_id, owner_id, fence, expires_at_ms) VALUES (?1, ?2, ?3, ?4)",
                params!["missing-project", "owner", 1, 0],
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn legacy_three_column_database_opens_without_rewriting_sentinel() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("legacy-tactus.db");
    let checksum = EXPECTED_TACTUS_V1_CHECKSUM.to_owned();
    let ledger_before = create_legacy_fixture(&database, EXPECTED_TACTUS_V1_CHECKSUM)?;

    let mut owner = RepositoryOwner::open(database.clone(), config()?, Duration::from_secs(2))?;
    owner
        .repository()
        .ensure_schema_ready(Duration::from_secs(1))?;
    owner.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    let ledger_after: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(ledger_after, ledger_before);
    let migration: (String, String) = connection.query_row(
        "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
        [1],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(migration, (TACTUS_V1_MIGRATION_NAME.to_owned(), checksum));
    assert_eq!(
        connection.query_row(
            "SELECT last_fence FROM projects WHERE project_id = ?1",
            ["legacy-sentinel"],
            |row| row.get::<_, i64>(0),
        )?,
        7
    );
    Ok(())
}

#[test]
fn closed_codecs_reject_unknown_negative_zero_and_non_boolean_values() {
    assert_eq!(MAX_WATCH_EVENTS, 1_000);
    assert_eq!(RunState::decode("state", "pending"), Ok(RunState::Pending));
    assert_eq!(
        CellState::decode("cell_state", "queued"),
        Ok(CellState::Queued)
    );
    assert_eq!(
        TransactionState::decode("transaction_state", "active"),
        Ok(TransactionState::Active)
    );
    assert_eq!(
        OutputStream::decode("stream", "stderr"),
        Ok(OutputStream::Stderr)
    );
    assert_eq!(
        CheckpointBackend::decode("backend", "git_aware"),
        Ok(CheckpointBackend::GitAware)
    );
    assert_eq!(
        RollbackFidelity::decode("fidelity", "declared_paths"),
        Ok(RollbackFidelity::DeclaredPaths)
    );
    assert_eq!(
        CheckpointEntryKind::decode("kind", "symlink"),
        Ok(CheckpointEntryKind::Symlink)
    );
    assert_eq!(
        RunState::decode("state", "future"),
        Err(CorruptStorageError::UnknownEnum { column: "state" })
    );
    assert_eq!(
        decode_non_negative("last_sequence", -1),
        Err(CorruptStorageError::NegativeInteger {
            column: "last_sequence"
        })
    );
    assert_eq!(
        decode_positive("revision", 0),
        Err(CorruptStorageError::ZeroInteger { column: "revision" })
    );
    assert_eq!(
        decode_boolean("is_executable", 2),
        Err(CorruptStorageError::InvalidBoolean {
            column: "is_executable"
        })
    );
    assert_eq!(
        decode_project_key("project_id", "not-a-uuid"),
        Err(CorruptStorageError::InvalidIdentifier {
            column: "project_id"
        })
    );
    assert_eq!(
        decode_digest("source_digest", "sha256:not-hex"),
        Err(CorruptStorageError::InvalidDigest {
            column: "source_digest"
        })
    );
    assert_eq!(
        decode_optional_blob("blob_digest", Some("sha256:not-hex"), "blob_length", None),
        Err(CorruptStorageError::PartialBlobReference {
            column: "blob_digest"
        })
    );
}
