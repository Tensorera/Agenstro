use std::{path::Path, time::Duration};

use segno_core::{
    AgentrodPort, CompileWorkflowPort, CompileWorkflowRequest, CompileWorkflowResponse,
    DispatchLookup, DispatchRequest, DispatchStart, LeaseOwnerId, OccurrenceId, PortError,
    ScheduleRevision, Sha256Digest, TaskId, UtcInstant, select_misfires,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ArchiveBudget, ArchiveError, CronEngine, OccurrenceStatus, PackageImporter, PublishedPackage,
    SqliteStore, StoreError, TaskListPage, TimeError,
    store::{DueSchedule, NewRevision},
};

const MAX_DISPATCH_CAPACITY: usize = 256;
const MAX_SCHEDULES_PER_TICK: usize = 200;
const MAX_MISFIRE_SCAN: usize = 10_000;
const MAX_MISFIRE_OUTPUT: usize = 1_000;
const MAX_LEASE_TTL: Duration = Duration::from_secs(60 * 60);

/// Complete bounded scheduler-loop configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    /// Dispatch claims per drive call.
    pub dispatch_capacity: usize,
    /// Lease TTL used for a short external dispatch call.
    pub lease_ttl: Duration,
    /// Due schedules evaluated in one tick.
    pub schedules_per_tick: usize,
    /// Maximum candidate instants examined for one downtime interval.
    pub misfire_scan_limit: usize,
    /// Maximum occurrences inserted from one schedule tick.
    pub misfire_output_limit: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            dispatch_capacity: 32,
            lease_ttl: Duration::from_secs(30),
            schedules_per_tick: 100,
            misfire_scan_limit: 1_000,
            misfire_output_limit: 100,
        }
    }
}

impl SchedulerConfig {
    /// Validates all queue/scan/lifecycle bounds.
    ///
    /// # Errors
    ///
    /// Rejects zero or hard-maximum-exceeding values.
    pub fn validate(self) -> Result<Self, SegnodError> {
        if self.dispatch_capacity == 0
            || self.dispatch_capacity > MAX_DISPATCH_CAPACITY
            || self.lease_ttl.is_zero()
            || self.lease_ttl > MAX_LEASE_TTL
            || self.schedules_per_tick == 0
            || self.schedules_per_tick > MAX_SCHEDULES_PER_TICK
            || self.misfire_scan_limit == 0
            || self.misfire_scan_limit > MAX_MISFIRE_SCAN
            || self.misfire_output_limit == 0
            || self.misfire_output_limit > MAX_MISFIRE_OUTPUT
        {
            return Err(SegnodError::InvalidConfiguration);
        }
        Ok(self)
    }
}

/// Result of publishing and registering one immutable task revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportResult {
    /// Stable task identity.
    pub task_id: String,
    /// Newly allocated revision.
    pub revision: u64,
    /// Immutable package digest.
    pub package_digest: String,
    /// Canonical versioned execution-spec digest.
    pub workflow_spec_digest: String,
}

/// Summary of one bounded dispatch/reconciliation drive call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DispatchBatch {
    /// Durable claims obtained.
    pub claimed: usize,
    /// Claims resolved to stable orchestration references.
    pub accepted: usize,
    /// Claims left pending because acceptance outcome is unknown.
    pub unknown: usize,
    /// Known pre-accept rejections recorded as recovery-required.
    pub rejected: usize,
}

/// Main synchronous application composition behind future authenticated RPC.
pub struct Segnod {
    importer: PackageImporter,
    store: SqliteStore,
    config: SchedulerConfig,
}

impl Segnod {
    /// Opens one independently owned Segno state root and SQLite database.
    ///
    /// # Errors
    ///
    /// Rejects relative state roots, invalid bounds, or package/store startup
    /// failures.
    pub fn open(
        state_root: &Path,
        archive_budget: ArchiveBudget,
        config: SchedulerConfig,
    ) -> Result<Self, SegnodError> {
        if !state_root.is_absolute() {
            return Err(SegnodError::StateRootNotAbsolute);
        }
        if is_unc_state_root(state_root) {
            return Err(SegnodError::UnsupportedStateFilesystem);
        }
        let config = config.validate()?;
        std::fs::create_dir_all(state_root)?;
        let importer = PackageImporter::new(state_root, archive_budget)?;
        let store = SqliteStore::open(&state_root.join("segno.sqlite3"))?;
        Ok(Self {
            importer,
            store,
            config,
        })
    }

    /// Publishes a portable ZIP and registers a disabled immutable revision.
    ///
    /// No package code is loaded or executed. The workflow is represented as a
    /// versioned frozen stage spec, but only Clef may compile it.
    ///
    /// # Errors
    ///
    /// Returns a package, time-policy, digest, or repository failure.
    pub fn import_package(
        &mut self,
        archive_path: &Path,
        now: UtcInstant,
    ) -> Result<ImportResult, SegnodError> {
        let published = self.importer.import(archive_path)?;
        let (task_id, policy) = published.manifest.validate()?;
        let _engine = CronEngine::new(&policy)?;
        let workflow_spec = frozen_workflow_spec(&published, &task_id)?;
        let workflow_spec_digest = digest_bytes(&workflow_spec);
        let revision = self.store.import_revision(
            NewRevision {
                task_id: task_id.clone(),
                name: published.manifest.name.clone(),
                package_digest: published.digest,
                workflow_spec_digest,
                workflow_spec,
                policy,
            },
            now,
        )?;
        Ok(ImportResult {
            task_id: task_id.as_str().to_owned(),
            revision: revision.value(),
            package_digest: published.digest.to_string(),
            workflow_spec_digest: workflow_spec_digest.to_string(),
        })
    }

    /// Compiles via the Clef-owned port, then enables exactly that revision.
    ///
    /// The external call occurs before the short enable transaction. A
    /// concurrent import makes the expected-revision update fail closed.
    ///
    /// # Errors
    ///
    /// Returns ID/revision, port, time, or store failures.
    pub fn enable<P: CompileWorkflowPort>(
        &mut self,
        task_id: &str,
        expected_revision: u64,
        compiler: &mut P,
        now: UtcInstant,
    ) -> Result<Sha256Digest, SegnodError> {
        let task_id = TaskId::parse(task_id).map_err(|_| SegnodError::InvalidIdentity)?;
        let revision =
            ScheduleRevision::new(expected_revision).map_err(|_| SegnodError::InvalidIdentity)?;
        let stored = self.store.revision_for_compile(&task_id, revision)?;
        let request = CompileWorkflowRequest::new(
            stored.task_id,
            stored.revision,
            stored.package_digest,
            stored.workflow_spec_digest,
            stored.workflow_spec,
        )?;
        let response = compiler.compile_workflow(&request)?;
        let engine = CronEngine::new(&stored.policy)?;
        let next_fire = engine
            .next_after(now)?
            .into_iter()
            .min()
            .ok_or(SegnodError::Time(TimeError::SearchExhausted))?;
        self.store
            .enable_revision(&task_id, revision, response.plan_digest, next_fire, now)?;
        Ok(response.plan_digest)
    }

    /// Returns a stable bounded task page.
    ///
    /// # Errors
    ///
    /// Returns a limit or repository failure.
    pub fn list_tasks(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<TaskListPage, SegnodError> {
        Ok(self.store.list_tasks(after, limit)?)
    }

    /// Creates a unique durable manual occurrence without executing a stage.
    ///
    /// # Errors
    ///
    /// Returns malformed task, uncompiled task, digest, or repository failure.
    pub fn run_now(&mut self, task_id: &str, now: UtcInstant) -> Result<OccurrenceId, SegnodError> {
        let task_id = TaskId::parse(task_id).map_err(|_| SegnodError::InvalidIdentity)?;
        Ok(self.store.create_manual_occurrence(&task_id, now, now)?)
    }

    /// Materializes due cron occurrences and advances durable `next_fire`.
    ///
    /// Work is bounded by schedules-per-tick, misfire scan, and selected output
    /// limits. Occurrence uniqueness remains a database constraint.
    ///
    /// # Errors
    ///
    /// Returns cron/time, policy-bound, or repository failures.
    pub fn tick(&mut self, now: UtcInstant) -> Result<Vec<OccurrenceId>, SegnodError> {
        let schedules = self
            .store
            .due_schedules(now, self.config.schedules_per_tick)?;
        let mut inserted = Vec::new();
        for schedule in schedules {
            let (due, next_fire) = collect_due(&schedule, now, self.config.misfire_scan_limit)?;
            let selected = select_misfires(
                &due,
                now,
                schedule.policy.misfire,
                self.config.misfire_output_limit,
            )?;
            inserted.extend(
                self.store
                    .advance_schedule(&schedule, &selected, next_fire, now)?,
            );
        }
        Ok(inserted)
    }

    /// Claims only available dispatch capacity and invokes `agentrod` outside
    /// SQLite transactions.
    ///
    /// # Errors
    ///
    /// Returns malformed owner, duration conversion, fence, or repository
    /// failures. Port rejection is durably summarized rather than returned as a
    /// partially applied transaction.
    pub fn dispatch_once<P: AgentrodPort>(
        &mut self,
        owner: &str,
        port: &mut P,
        now: UtcInstant,
    ) -> Result<DispatchBatch, SegnodError> {
        let owner = LeaseOwnerId::parse(owner).map_err(|_| SegnodError::InvalidIdentity)?;
        let ttl_ms = u64::try_from(self.config.lease_ttl.as_millis())
            .map_err(|_| SegnodError::InvalidConfiguration)?;
        let claims = self
            .store
            .claim_due(&owner, now, ttl_ms, self.config.dispatch_capacity)?;
        self.drive_claims(claims, port, now, false)
    }

    /// Reconciles uncertain dispatches after lease expiry/restart by query first
    /// and then idempotent replay of the same occurrence key when absent.
    ///
    /// # Errors
    ///
    /// Returns malformed owner, duration conversion, fence, or repository
    /// failures.
    pub fn reconcile_once<P: AgentrodPort>(
        &mut self,
        owner: &str,
        port: &mut P,
        now: UtcInstant,
    ) -> Result<DispatchBatch, SegnodError> {
        let owner = LeaseOwnerId::parse(owner).map_err(|_| SegnodError::InvalidIdentity)?;
        let ttl_ms = u64::try_from(self.config.lease_ttl.as_millis())
            .map_err(|_| SegnodError::InvalidConfiguration)?;
        let claims =
            self.store
                .claim_reconciliation(&owner, now, ttl_ms, self.config.dispatch_capacity)?;
        self.drive_claims(claims, port, now, true)
    }

    fn drive_claims<P: AgentrodPort>(
        &mut self,
        claims: Vec<DispatchRequest>,
        port: &mut P,
        now: UtcInstant,
        query_first: bool,
    ) -> Result<DispatchBatch, SegnodError> {
        let mut batch = DispatchBatch {
            claimed: claims.len(),
            ..DispatchBatch::default()
        };
        for claim in claims {
            let start = if query_first {
                match port.query_by_occurrence(&claim.occurrence_id) {
                    Ok(DispatchLookup::Found(run_id)) => DispatchStart::Accepted(run_id),
                    Ok(DispatchLookup::NotFound) => match port.start_workflow(&claim) {
                        Ok(value) => value,
                        Err(error) => {
                            self.record_port_failure(&claim, &error, now)?;
                            batch.rejected += 1;
                            continue;
                        }
                    },
                    Err(_) => {
                        batch.unknown += 1;
                        continue;
                    }
                }
            } else {
                match port.start_workflow(&claim) {
                    Ok(value) => value,
                    Err(error) => {
                        self.record_port_failure(&claim, &error, now)?;
                        batch.rejected += 1;
                        continue;
                    }
                }
            };
            match start {
                DispatchStart::Accepted(run_id) => {
                    self.store.record_dispatch(&claim, &run_id, now)?;
                    batch.accepted += 1;
                }
                DispatchStart::OutcomeUnknown => batch.unknown += 1,
            }
        }
        Ok(batch)
    }

    fn record_port_failure(
        &mut self,
        claim: &DispatchRequest,
        error: &PortError,
        now: UtcInstant,
    ) -> Result<(), SegnodError> {
        let code = match error {
            PortError::InvalidRequest => "agentrod_invalid_request",
            PortError::Unavailable => "agentrod_unavailable",
            PortError::Rejected(_) => "agentrod_rejected",
        };
        self.store.record_dispatch_failure(claim, code, now)?;
        Ok(())
    }

    /// Returns bounded status for one occurrence.
    ///
    /// # Errors
    ///
    /// Returns malformed identity, not found, corrupt state, or SQLite failure.
    pub fn status(&self, occurrence_id: &str) -> Result<OccurrenceStatus, SegnodError> {
        let occurrence_id =
            OccurrenceId::parse(occurrence_id).map_err(|_| SegnodError::InvalidIdentity)?;
        Ok(self.store.occurrence_status(&occurrence_id)?)
    }

    /// Records a terminal bounded summary received from `agentrod` query/event.
    ///
    /// This releases task-level overlap admission but does not copy workflow
    /// logs or artifact metadata into Segno.
    ///
    /// # Errors
    ///
    /// Returns malformed identity/code, state conflict, or SQLite failure.
    pub fn record_terminal_summary(
        &mut self,
        occurrence_id: &str,
        succeeded: bool,
        code: &str,
        now: UtcInstant,
    ) -> Result<(), SegnodError> {
        let occurrence_id =
            OccurrenceId::parse(occurrence_id).map_err(|_| SegnodError::InvalidIdentity)?;
        self.store
            .record_terminal_summary(&occurrence_id, succeeded, code, now)?;
        Ok(())
    }

    /// Flushes a bounded passive WAL checkpoint before owner shutdown.
    ///
    /// # Errors
    ///
    /// Returns a SQLite checkpoint failure.
    pub fn shutdown(self) -> Result<(), SegnodError> {
        self.store.checkpoint()?;
        Ok(())
    }
}

fn collect_due(
    schedule: &DueSchedule,
    now: UtcInstant,
    scan_limit: usize,
) -> Result<(Vec<UtcInstant>, UtcInstant), SegnodError> {
    let engine = CronEngine::new(&schedule.policy)?;
    let mut due = vec![schedule.next_fire];
    let mut cursor = schedule.next_fire;
    for _ in 0..scan_limit {
        let next = engine.next_after(cursor)?;
        let mut future = None;
        let mut advanced = false;
        for instant in next {
            if instant <= cursor {
                continue;
            }
            advanced = true;
            if instant <= now {
                if due.last() != Some(&instant) {
                    due.push(instant);
                }
                cursor = instant;
            } else {
                future = Some(future.map_or(instant, |current: UtcInstant| current.min(instant)));
            }
        }
        if let Some(next_fire) = future {
            return Ok((due, next_fire));
        }
        if !advanced {
            return Err(SegnodError::MisfireScanExhausted);
        }
    }
    Err(SegnodError::MisfireScanExhausted)
}

fn frozen_workflow_spec(
    package: &PublishedPackage,
    task_id: &TaskId,
) -> Result<Vec<u8>, SegnodError> {
    let mut output = Vec::with_capacity(512);
    output.extend_from_slice(b"segno-clef-execution-spec-v1\0");
    push_field(
        &mut output,
        "package_digest",
        package.digest.to_string().as_bytes(),
    )?;
    push_field(
        &mut output,
        "post",
        package.manifest.scripts.post.as_bytes(),
    )?;
    push_field(&mut output, "pre", package.manifest.scripts.pre.as_bytes())?;
    push_field(
        &mut output,
        "main",
        package.manifest.scripts.main.as_bytes(),
    )?;
    push_field(&mut output, "task_id", task_id.as_str().as_bytes())?;
    Ok(output)
}

fn push_field(output: &mut Vec<u8>, name: &str, value: &[u8]) -> Result<(), SegnodError> {
    let name_length = u16::try_from(name.len()).map_err(|_| SegnodError::WorkflowSpec)?;
    let value_length = u64::try_from(value.len()).map_err(|_| SegnodError::WorkflowSpec)?;
    output.extend_from_slice(&name_length.to_be_bytes());
    output.extend_from_slice(name.as_bytes());
    output.extend_from_slice(&value_length.to_be_bytes());
    output.extend_from_slice(value);
    if output.len() > 1024 * 1024 {
        return Err(SegnodError::WorkflowSpec);
    }
    Ok(())
}

fn digest_bytes(value: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(value);
    Sha256Digest::from_bytes(hasher.finalize().into())
}

#[cfg(windows)]
fn is_unc_state_root(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(
                prefix.kind(),
                Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _) | Prefix::DeviceNS(_)
            )
    )
}

#[cfg(not(windows))]
const fn is_unc_state_root(_path: &Path) -> bool {
    false
}

/// Compiler adapter for CLI composition after a supervisor obtains a verified
/// plan digest from `agentrod`.
pub struct StaticCompiler {
    plan_digest: Sha256Digest,
}

impl StaticCompiler {
    /// Creates a pinned response adapter.
    #[must_use]
    pub const fn new(plan_digest: Sha256Digest) -> Self {
        Self { plan_digest }
    }
}

impl CompileWorkflowPort for StaticCompiler {
    fn compile_workflow(
        &mut self,
        _request: &CompileWorkflowRequest,
    ) -> Result<CompileWorkflowResponse, PortError> {
        Ok(CompileWorkflowResponse {
            plan_digest: self.plan_digest,
        })
    }
}

/// Segno application failure.
#[derive(Debug, Error)]
pub enum SegnodError {
    /// State root must be absolute.
    #[error("Segno state root must be absolute")]
    StateRootNotAbsolute,
    /// UNC/device state paths cannot provide the selected SQLite WAL contract.
    #[error("Segno state root uses an unsupported network/device path")]
    UnsupportedStateFilesystem,
    /// Scheduler bounds are zero or excessive.
    #[error("scheduler configuration is invalid")]
    InvalidConfiguration,
    /// Task/occurrence/revision identity is malformed.
    #[error("Segno identity is invalid")]
    InvalidIdentity,
    /// Canonical execution-spec encoding exceeded its bound.
    #[error("frozen workflow spec is invalid or excessive")]
    WorkflowSpec,
    /// Downtime candidate scan reached its hard bound.
    #[error("misfire scan budget was exhausted")]
    MisfireScanExhausted,
    /// Filesystem operation failed.
    #[error("Segno state filesystem operation failed")]
    Io(#[from] std::io::Error),
    /// Package import failed.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// Time policy failed.
    #[error(transparent)]
    Time(#[from] TimeError),
    /// SQLite repository failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Clef/agentrod port failed before state commit.
    #[error(transparent)]
    Port(#[from] PortError),
    /// Core lease/misfire invariant failed.
    #[error("Segno scheduling invariant failed")]
    Core(#[from] segno_core::LeaseError),
}
