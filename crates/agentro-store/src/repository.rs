use std::{path::Path, time::Duration};

use rusqlite::Connection;

use crate::{
    actor::{JournalMode, StoreError},
    migration::{self, Migration, MigrationProfile},
};

pub(crate) fn open(
    path: &Path,
    busy_timeout: Duration,
    journal_mode: JournalMode,
    migrations: &[Migration],
    profile: MigrationProfile,
) -> Result<Connection, StoreError> {
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(busy_timeout)?;
    connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")?;
    let requested_mode = match journal_mode {
        JournalMode::Wal => "PRAGMA journal_mode = WAL",
        JournalMode::Delete => "PRAGMA journal_mode = DELETE",
    };
    let actual_mode: String = connection.query_row(requested_mode, [], |row| row.get(0))?;
    let expected_mode = match journal_mode {
        JournalMode::Wal => "wal",
        JournalMode::Delete => "delete",
    };
    if !actual_mode.eq_ignore_ascii_case(expected_mode) {
        return Err(StoreError::PragmaNotApplied {
            pragma: "journal_mode",
            actual: bounded_pragma_value(&actual_mode),
        });
    }

    let foreign_keys: u32 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if foreign_keys != 1 {
        return Err(StoreError::PragmaNotApplied {
            pragma: "foreign_keys",
            actual: foreign_keys.to_string(),
        });
    }
    let synchronous: u32 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    if synchronous != 2 {
        return Err(StoreError::PragmaNotApplied {
            pragma: "synchronous",
            actual: synchronous.to_string(),
        });
    }
    let busy_timeout_ms: i64 = connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
    let expected_busy_timeout_ms = i64::try_from(busy_timeout.as_millis()).unwrap_or(i64::MAX);
    if busy_timeout_ms != expected_busy_timeout_ms {
        return Err(StoreError::PragmaNotApplied {
            pragma: "busy_timeout",
            actual: busy_timeout_ms.to_string(),
        });
    }

    quick_check(&connection)?;
    migration::initialize_and_apply(&mut connection, migrations, profile)?;
    quick_check(&connection)?;
    Ok(connection)
}

pub(crate) fn checkpoint_on_shutdown(
    connection: &Connection,
    journal_mode: JournalMode,
) -> Result<(), StoreError> {
    if journal_mode == JournalMode::Wal {
        let _: (u32, u32, u32) =
            connection.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
    }
    Ok(())
}

fn bounded_pragma_value(value: &str) -> String {
    value.chars().take(64).collect()
}

fn quick_check(connection: &Connection) -> Result<(), StoreError> {
    let diagnostic: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if diagnostic == "ok" {
        Ok(())
    } else {
        Err(StoreError::QuickCheckFailed {
            diagnostic: diagnostic.chars().take(256).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, time::Duration};

    use tempfile::tempdir;

    use super::*;
    use crate::{StoreActor, StoreConfig};

    #[test]
    fn sqlite_busy_is_bounded_and_actor_remains_usable() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let database = temporary.path().join("service.db");
        let config = StoreConfig::new(4, Duration::from_millis(50), JournalMode::Wal)?;
        let mut actor =
            StoreActor::start(database.clone(), config, Vec::new(), Duration::from_secs(2))?;
        let locker = Connection::open(database)?;
        locker.execute_batch("BEGIN IMMEDIATE")?;
        let error = match actor.handle().call(Duration::from_secs(1), |connection| {
            connection.execute_batch("CREATE TABLE blocked (id INTEGER PRIMARY KEY)")?;
            Ok(())
        }) {
            Ok(()) => return Err("write unexpectedly bypassed the held SQLite lock".into()),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StoreError::Sqlite { ref source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
        ));
        locker.execute_batch("ROLLBACK")?;
        assert_eq!(actor.handle().schema_version(Duration::from_secs(1))?, 0);
        actor.shutdown(Duration::from_secs(2))?;
        Ok(())
    }
}
