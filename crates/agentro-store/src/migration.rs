use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::actor::StoreError;

/// Hard maximum migration count accepted by one service schema.
pub const MAX_MIGRATIONS: usize = 1_024;
/// Hard maximum SQL bytes in one migration.
pub const MAX_MIGRATION_SQL_BYTES: usize = 1_048_576;
const MAX_MIGRATION_NAME_BYTES: usize = 128;
const AGENTRO_LEDGER_SQL: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY CHECK (version > 0),
            name TEXT NOT NULL UNIQUE,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );";
const TACTUS_V1_LEDGER_SQL: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY CHECK (version > 0),
            name TEXT NOT NULL UNIQUE,
            checksum TEXT NOT NULL
        );";
const AGENTRO_INSERT_SQL: &str =
    "INSERT INTO schema_migrations (version, name, checksum, applied_at)
             VALUES (?1, ?2, ?3, CAST(strftime('%s', 'now') AS INTEGER))";
const TACTUS_V1_INSERT_SQL: &str =
    "INSERT INTO schema_migrations (version, name, checksum) VALUES (?1, ?2, ?3)";
const AGENTRO_CHECKSUM_DOMAIN: &[u8] = b"agentro.sqlite.migration.v1\0";
const TACTUS_V1_CHECKSUM_DOMAIN: &[u8] = b"tactus.sqlite.migration.v1\0";

/// Closed migration-ledger and checksum compatibility behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum MigrationProfile {
    /// The default Agentro four-column ledger and checksum domain.
    #[default]
    AgentroV1,
    /// The legacy Tactus three-column ledger and checksum domain.
    ///
    /// This profile does not alter an existing ledger or synthesize an
    /// `applied_at` column. Its sole purpose is opening and extending databases
    /// created by the original Tactus v1 migration algorithm.
    TactusV1Compatibility,
}

impl MigrationProfile {
    fn ledger_sql(self) -> &'static str {
        match self {
            Self::AgentroV1 => AGENTRO_LEDGER_SQL,
            Self::TactusV1Compatibility => TACTUS_V1_LEDGER_SQL,
        }
    }

    fn insert_sql(self) -> &'static str {
        match self {
            Self::AgentroV1 => AGENTRO_INSERT_SQL,
            Self::TactusV1Compatibility => TACTUS_V1_INSERT_SQL,
        }
    }

    fn checksum_domain(self) -> &'static [u8] {
        match self {
            Self::AgentroV1 => AGENTRO_CHECKSUM_DOMAIN,
            Self::TactusV1Compatibility => TACTUS_V1_CHECKSUM_DOMAIN,
        }
    }
}

/// Invalid immutable migration definition.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum MigrationDefinitionError {
    /// Migration versions start at one.
    #[error("migration version must be greater than zero")]
    ZeroVersion,
    /// Migration names use a small stable ASCII alphabet.
    #[error("migration name is empty, too long, or contains unsupported characters")]
    InvalidName,
    /// Migration SQL must be non-empty and bounded.
    #[error("migration SQL is empty or exceeds {maximum} bytes")]
    InvalidSql {
        /// Maximum accepted SQL bytes.
        maximum: usize,
    },
}

/// One immutable, monotonically numbered schema migration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

impl Migration {
    /// Constructs a validated migration definition.
    ///
    /// # Errors
    ///
    /// Returns [`MigrationDefinitionError`] for zero versions, unstable names,
    /// or empty/excessive SQL.
    pub fn new(
        version: u32,
        name: &'static str,
        sql: &'static str,
    ) -> Result<Self, MigrationDefinitionError> {
        if version == 0 {
            return Err(MigrationDefinitionError::ZeroVersion);
        }
        if name.is_empty()
            || name.len() > MAX_MIGRATION_NAME_BYTES
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(MigrationDefinitionError::InvalidName);
        }
        if sql.trim().is_empty() || sql.len() > MAX_MIGRATION_SQL_BYTES {
            return Err(MigrationDefinitionError::InvalidSql {
                maximum: MAX_MIGRATION_SQL_BYTES,
            });
        }
        Ok(Self { version, name, sql })
    }

    pub(crate) fn version(self) -> u32 {
        self.version
    }

    pub(crate) fn name(self) -> &'static str {
        self.name
    }

    pub(crate) fn sql(self) -> &'static str {
        self.sql
    }
}

pub(crate) fn validate_plan(migrations: &[Migration]) -> Result<(), StoreError> {
    if migrations.len() > MAX_MIGRATIONS {
        return Err(StoreError::InvalidMigrationPlan {
            maximum: MAX_MIGRATIONS,
        });
    }
    for (index, migration) in migrations.iter().enumerate() {
        let expected = u32::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1));
        if expected != Some(migration.version()) {
            return Err(StoreError::InvalidMigrationPlan {
                maximum: MAX_MIGRATIONS,
            });
        }
    }
    Ok(())
}

pub(crate) fn initialize_and_apply(
    connection: &mut Connection,
    migrations: &[Migration],
    profile: MigrationProfile,
) -> Result<(), StoreError> {
    connection.execute_batch(profile.ledger_sql())?;
    let (row_count, database_version): (u32, u32) = connection.query_row(
        "SELECT COUNT(version), COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let supported = migrations.last().map_or(0, |migration| migration.version());
    if usize::try_from(row_count).unwrap_or(usize::MAX) > MAX_MIGRATIONS {
        return Err(StoreError::InvalidMigrationPlan {
            maximum: MAX_MIGRATIONS,
        });
    }
    if database_version > supported {
        return Err(StoreError::DatabaseSchemaTooNew {
            database: database_version,
            supported,
        });
    }

    for migration in migrations.iter().take(database_version as usize) {
        let applied: Option<(String, String)> = connection
            .query_row(
                "SELECT name, checksum FROM schema_migrations WHERE version = ?1",
                [migration.version()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (name, checksum) = applied.ok_or(StoreError::MissingAppliedMigration {
            version: migration.version(),
        })?;
        if name != migration.name() {
            return Err(StoreError::MigrationNameMismatch {
                version: migration.version(),
            });
        }
        if checksum != migration_checksum(*migration, profile) {
            return Err(StoreError::MigrationChecksumMismatch {
                version: migration.version(),
            });
        }
    }

    for migration in migrations.iter().skip(database_version as usize) {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(migration.sql())?;
        transaction.execute(
            profile.insert_sql(),
            params![
                migration.version(),
                migration.name(),
                migration_checksum(*migration, profile)
            ],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

pub(crate) fn schema_version(connection: &Connection) -> Result<u32, StoreError> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

fn migration_checksum(migration: Migration, profile: MigrationProfile) -> String {
    let mut hasher = Sha256::new();
    hasher.update(profile.checksum_domain());
    hasher.update(migration.version().to_be_bytes());
    hasher.update(migration.name().as_bytes());
    hasher.update([0]);
    hasher.update(migration.sql().as_bytes());
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}
