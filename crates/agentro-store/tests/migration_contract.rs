use std::{error::Error, time::Duration};

use agentro_store::{JournalMode, Migration, StoreActor, StoreConfig, StoreError};
use rusqlite::Connection;
use tempfile::tempdir;

const CREATE_ITEMS: &str =
    "CREATE TABLE legacy_items (id INTEGER PRIMARY KEY, value TEXT NOT NULL);";
const AGENTRO_CHECKSUM: &str = "3312a1a9450b320f9949e4bbcbc22a19bf8123c63061abb2312773d5b90e8343";

fn config() -> Result<StoreConfig, StoreError> {
    StoreConfig::new(8, Duration::from_millis(250), JournalMode::Wal)
}

fn migration() -> Result<Migration, Box<dyn Error>> {
    Ok(Migration::new(1, "create_items", CREATE_ITEMS)?)
}

#[test]
fn default_migration_uses_the_agentro_ledger_and_checksum_domain() -> Result<(), Box<dyn Error>> {
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
    let mut statement = connection.prepare("PRAGMA table_info(schema_migrations)")?;
    let columns: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    assert_eq!(columns, ["version", "name", "checksum", "applied_at"]);

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
