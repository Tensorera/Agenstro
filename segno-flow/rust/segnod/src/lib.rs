//! Durable Segno package import, SQLite scheduling, and `agentrod` composition.
//!
//! The application persists intent before external calls, admits only a
//! configured dispatch batch, and reconciles uncertain responses by querying or
//! safely replaying the same occurrence idempotency key. It never starts a task
//! process and stores only bounded orchestration summaries.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod archive;
mod service;
mod store;
mod time;

pub use archive::{
    ArchiveBudget, ArchiveError, PackageImporter, PackageManifest, PublishedPackage,
    ScheduleManifest, ScriptsManifest,
};
pub use service::{
    DispatchBatch, ImportResult, SchedulerConfig, Segnod, SegnodError, StaticCompiler,
};
pub use store::{OccurrenceStatus, SqliteStore, StoreError, TaskListPage, TaskSummary};
pub use time::{CronEngine, TimeError, resolve_local};
