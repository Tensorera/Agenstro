use std::{error::Error, path::Path, time::Duration};

use agentro_store::{
    JournalMode, Migration, MigrationProfile, StoreActor, StoreConfig, StoreError,
};
use rusqlite::{Connection, params};
use tempfile::tempdir;

const CREATE_ITEMS: &str =
    "CREATE TABLE legacy_items (id INTEGER PRIMARY KEY, value TEXT NOT NULL);";
const AGENTRO_CHECKSUM: &str = "3312a1a9450b320f9949e4bbcbc22a19bf8123c63061abb2312773d5b90e8343";
const TACTUS_CHECKSUM: &str = "05e328096f14a50f8a4faa46db1985e3c252f496f01adf346e27c66c9592fc2d";

fn config() -> Result<StoreConfig, StoreError> {
    StoreConfig::new(8, Duration::from_millis(250), JournalMode::Wal)
}

fn migration() -> Result<Migration, Box<dyn Error>> {
    Ok(Migration::new(1, "create_items", CREATE_ITEMS)?)
}

fn start_tactus(database: &Path, migrations: Vec<Migration>) -> Result<StoreActor, StoreError> {
    StoreActor::start_with_migration_profile(
        database.to_path_buf(),
        config()?,
        migrations,
        MigrationProfile::TactusV1Compatibility,
        Duration::from_secs(2),
    )
}

fn ledger_columns(connection: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = connection.prepare("PRAGMA table_info(schema_migrations)")?;
    statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect()
}

fn create_tactus_fixture(
    database: &Path,
    version: u32,
    checksum: &str,
) -> Result<(), Box<dyn Error>> {
    let connection = Connection::open(database)?;
    connection.execute_batch(
        "CREATE TABLE schema_migrations (
            version INTEGER PRIMARY KEY CHECK (version > 0),
            name TEXT NOT NULL UNIQUE,
            checksum TEXT NOT NULL
        );
        CREATE TABLE legacy_items (id INTEGER PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    connection.execute(
        "INSERT INTO schema_migrations (version, name, checksum) VALUES (?1, ?2, ?3)",
        params![version, "create_items", checksum],
    )?;
    Ok(())
}

fn startup_error(result: Result<StoreActor, StoreError>) -> Result<StoreError, Box<dyn Error>> {
    match result {
        Ok(mut actor) => {
            actor.shutdown(Duration::from_secs(2))?;
            Err("database unexpectedly opened".into())
        }
        Err(error) => Ok(error),
    }
}

#[test]
fn default_profile_preserves_four_column_agentro_ledger() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("default.db");
    let mut actor = StoreActor::start(
        database.clone(),
        config()?,
        vec![migration()?],
        Duration::from_secs(2),
    )?;
    actor.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        ledger_columns(&connection)?,
        ["version", "name", "checksum", "applied_at"]
    );
    let row: (u32, String, String, i64) = connection.query_row(
        "SELECT version, name, checksum, applied_at FROM schema_migrations WHERE version = ?1",
        [1],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        (row.0, row.1.as_str(), row.2.as_str()),
        (1, "create_items", AGENTRO_CHECKSUM)
    );
    assert!(row.3 > 0);
    Ok(())
}

#[test]
fn tactus_profile_opens_legacy_fixture_without_rewriting_ledger() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("tactus.db");
    create_tactus_fixture(&database, 1, TACTUS_CHECKSUM)?;
    let before_schema: String = Connection::open(&database)?.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |row| row.get(0),
    )?;

    let mut actor = start_tactus(&database, vec![migration()?])?;
    assert_eq!(actor.handle().schema_version(Duration::from_secs(1))?, 1);
    actor.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        ledger_columns(&connection)?,
        ["version", "name", "checksum"]
    );
    let after_schema: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(after_schema, before_schema);
    let row: (String, String) = connection.query_row(
        "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
        [1],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(row, ("create_items".to_owned(), TACTUS_CHECKSUM.to_owned()));
    Ok(())
}

#[test]
fn tactus_profile_creates_original_three_column_ledger() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("new-tactus.db");
    let mut actor = start_tactus(&database, vec![migration()?])?;
    actor.shutdown(Duration::from_secs(2))?;

    let connection = Connection::open(database)?;
    assert_eq!(
        ledger_columns(&connection)?,
        ["version", "name", "checksum"]
    );
    let checksum: String = connection.query_row(
        "SELECT checksum FROM schema_migrations WHERE version = ?1",
        [1],
        |row| row.get(0),
    )?;
    assert_eq!(checksum, TACTUS_CHECKSUM);
    Ok(())
}

#[test]
fn tactus_profile_rejects_checksum_mismatch() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("changed.db");
    create_tactus_fixture(&database, 1, AGENTRO_CHECKSUM)?;

    let error = startup_error(start_tactus(&database, vec![migration()?]))?;
    assert!(matches!(
        error,
        StoreError::MigrationChecksumMismatch { version: 1 }
    ));
    Ok(())
}

#[test]
fn tactus_profile_rejects_default_database_without_mutating_its_ledger()
-> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("default-remains-default.db");
    let mut actor = StoreActor::start(
        database.clone(),
        config()?,
        vec![migration()?],
        Duration::from_secs(2),
    )?;
    actor.shutdown(Duration::from_secs(2))?;
    let before: (String, String) = Connection::open(&database)?.query_row(
        "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
        [1],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let error = startup_error(start_tactus(&database, vec![migration()?]))?;
    assert!(matches!(
        error,
        StoreError::MigrationChecksumMismatch { version: 1 }
    ));
    let connection = Connection::open(database)?;
    assert_eq!(
        ledger_columns(&connection)?,
        ["version", "name", "checksum", "applied_at"]
    );
    let after: (String, String) = connection.query_row(
        "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
        [1],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(after, before);
    Ok(())
}

#[test]
fn tactus_profile_rejects_schema_newer_than_plan() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let database = temporary.path().join("newer.db");
    create_tactus_fixture(&database, 2, TACTUS_CHECKSUM)?;

    let error = startup_error(start_tactus(&database, vec![migration()?]))?;
    assert!(matches!(
        error,
        StoreError::DatabaseSchemaTooNew {
            database: 2,
            supported: 1
        }
    ));
    Ok(())
}
