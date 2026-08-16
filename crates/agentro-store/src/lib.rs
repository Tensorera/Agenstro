//! Bounded single-writer SQLite and migration primitives.
//!
//! One named actor thread exclusively owns each `rusqlite::Connection`.
//! Cloneable handles can submit typed closures to a bounded queue, but only the
//! [`StoreActor`] owner can close admission and join the writer. Migration SQL
//! is ordered, checksummed, and applied in short immediate transactions.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod actor;
mod migration;
mod repository;
pub mod tactus;

pub use actor::{
    JournalMode, MAX_BUSY_TIMEOUT, MAX_QUEUE_CAPACITY, StoreActor, StoreConfig, StoreError,
    StoreHandle,
};
pub use migration::{
    MAX_MIGRATION_SQL_BYTES, MAX_MIGRATIONS, Migration, MigrationDefinitionError, MigrationProfile,
};
