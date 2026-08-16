use std::{
    collections::BTreeMap,
    error::Error,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agentro_contracts::{CanonicalHasher, DigestError, RequestId, Sha256Digest};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    BlobRef, CancellationSource, CancellationToken, CellId, CellState, CheckpointError,
    FencingToken, LeaseOwnerId, OutputStream, ProjectId, RunId, RunState, TransactionState,
    WorkerCommand, WorkerCompletion, WorkerError, WorkerEventSink, WorkerPort, WorkerTerminal,
    WorkspacePort, WorkspaceTransactionId,
    checkpoint::workspace_binding_digest,
    store::{
        BeginIntent, FinishDisposition, JournalMode, LeaseGrant, Repository, RepositoryError,
        RepositoryOwner, RunRecord, StoredEvent,
    },
};

const MAX_ACTIVE_RUNS: usize = 64;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_DURABLE_OUTPUT_BYTES: u64 = 1 << 30;
const MAX_DURABLE_OUTPUT_RECORDS: u64 = 1_000_000;
const MAX_LEASE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const REAP_POLL: Duration = Duration::from_millis(5);

/// Time source used for durable timestamps and lease decisions.
pub trait Clock: Send + Sync {
    /// Returns milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

/// Production wall clock. Durations and process deadlines remain monotonic in
/// the underlying supervisor; this clock is used only for durable instants.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

/// Deterministic clock for contract tests and embedded callers.
#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicU64,
}

impl ManualClock {
    /// Creates a clock at an explicit durable instant.
    #[must_use]
    pub const fn new(now_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(now_ms),
        }
    }

    /// Replaces the current durable instant.
    pub fn set(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::Release);
    }

    /// Advances the clock with saturating arithmetic.
    pub fn advance(&self, duration: Duration) {
        let delta = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let _ = self
            .now_ms
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_add(delta))
            });
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::Acquire)
    }
}

/// SQLite journal selection made after the caller checks the state filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateDurability {
    /// Full-synchronous WAL on a confirmed local filesystem.
    LocalWal,
    /// Full-synchronous rollback journal when WAL support is not asserted.
    ConservativeDelete,
}

/// Bounded daemon ownership, storage, output, and lease settings.
#[derive(Clone, Copy, Debug)]
pub struct DaemonConfig {
    max_active_runs: usize,
    max_source_bytes: usize,
    max_output_bytes: u64,
    max_output_records: u64,
    execution_lease_ttl: Duration,
    durability: StateDurability,
}

impl DaemonConfig {
    /// Constructs complete resource bounds for the vertical runtime.
    ///
    /// # Errors
    ///
    /// Returns [`TactusError::InvalidConfiguration`] for a zero or excessive
    /// value.
    pub fn new(
        max_active_runs: usize,
        max_source_bytes: usize,
        max_output_bytes: u64,
        max_output_records: u64,
        execution_lease_ttl: Duration,
        durability: StateDurability,
    ) -> Result<Self, TactusError> {
        if max_active_runs == 0 || max_active_runs > MAX_ACTIVE_RUNS {
            return Err(TactusError::InvalidConfiguration {
                field: "active runs",
            });
        }
        if max_source_bytes == 0 || max_source_bytes > MAX_SOURCE_BYTES {
            return Err(TactusError::InvalidConfiguration {
                field: "source bytes",
            });
        }
        if max_output_bytes == 0 || max_output_bytes > MAX_DURABLE_OUTPUT_BYTES {
            return Err(TactusError::InvalidConfiguration {
                field: "output bytes",
            });
        }
        if max_output_records == 0 || max_output_records > MAX_DURABLE_OUTPUT_RECORDS {
            return Err(TactusError::InvalidConfiguration {
                field: "output records",
            });
        }
        if execution_lease_ttl.is_zero() || execution_lease_ttl > MAX_LEASE_TTL {
            return Err(TactusError::InvalidConfiguration {
                field: "execution lease TTL",
            });
        }
        Ok(Self {
            max_active_runs,
            max_source_bytes,
            max_output_bytes,
            max_output_records,
            execution_lease_ttl,
            durability,
        })
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            max_active_runs: 4,
            max_source_bytes: MAX_SOURCE_BYTES,
            max_output_bytes: 64 * 1024 * 1024,
            max_output_records: 100_000,
            execution_lease_ttl: Duration::from_secs(60 * 60),
            durability: StateDurability::LocalWal,
        }
    }
}

/// Idempotent begin-workspace command accepted by `tactusd` composition.
#[derive(Clone, Debug)]
pub struct BeginRequest {
    request_id: RequestId,
    project_id: ProjectId,
    cell_id: CellId,
    source: Vec<u8>,
    workspace_root: PathBuf,
    lease_owner_id: LeaseOwnerId,
    lease_ttl: Duration,
}

impl BeginRequest {
    /// Creates a begin command. Bounds and the absolute workspace binding are
    /// validated before durable intent is written.
    #[must_use]
    pub fn new(
        request_id: RequestId,
        project_id: ProjectId,
        cell_id: CellId,
        source: Vec<u8>,
        workspace_root: PathBuf,
        lease_owner_id: LeaseOwnerId,
        lease_ttl: Duration,
    ) -> Self {
        Self {
            request_id,
            project_id,
            cell_id,
            source,
            workspace_root,
            lease_owner_id,
            lease_ttl,
        }
    }
}

/// Asynchronous execute acceptance command for a durable pending run.
#[derive(Clone, Debug)]
pub struct ExecuteRequest {
    run_id: RunId,
    workspace_root: PathBuf,
}

impl ExecuteRequest {
    /// Rebinds a durable run to an explicit workspace root.
    #[must_use]
    pub const fn new(run_id: RunId, workspace_root: PathBuf) -> Self {
        Self {
            run_id,
            workspace_root,
        }
    }
}

/// Stable event kinds returned by bounded watch pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunEventKind {
    /// Durable run and transaction intent was created.
    IntentCreated,
    /// Source and baseline checkpoint are durable.
    WorkspaceReady,
    /// A supervised worker started.
    Running,
    /// One output chunk was published to CAS.
    Output,
    /// Cancellation was durably requested.
    CancelRequested,
    /// Result checkpoint and success committed.
    Succeeded,
    /// Execution failed.
    Failed,
    /// Cancellation completed.
    Cancelled,
    /// Startup reconciliation began.
    Recovering,
    /// A prior worker could not be resumed.
    Interrupted,
    /// Workspace consistency could not be proven.
    WorkspaceConflict,
}

impl RunEventKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "intent_created" => Some(Self::IntentCreated),
            "workspace_ready" => Some(Self::WorkspaceReady),
            "running" => Some(Self::Running),
            "output" => Some(Self::Output),
            "cancel_requested" => Some(Self::CancelRequested),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "recovering" => Some(Self::Recovering),
            "interrupted" => Some(Self::Interrupted),
            "workspace_conflict" => Some(Self::WorkspaceConflict),
            _ => None,
        }
    }
}

/// One durable event with optional chunk reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEvent {
    sequence: u64,
    kind: RunEventKind,
    worker_sequence: Option<u64>,
    stream: Option<OutputStream>,
    blob: Option<BlobRef>,
    occurred_at_ms: u64,
}

impl RunEvent {
    /// Returns the run-local durable sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the stable event kind.
    #[must_use]
    pub const fn kind(&self) -> RunEventKind {
        self.kind
    }

    /// Returns the worker-local sequence for an output event.
    #[must_use]
    pub const fn worker_sequence(&self) -> Option<u64> {
        self.worker_sequence
    }

    /// Returns the independent output stream for a chunk event.
    #[must_use]
    pub const fn stream(&self) -> Option<OutputStream> {
        self.stream
    }

    /// Returns the CAS chunk reference, never inline unbounded output.
    #[must_use]
    pub const fn blob(&self) -> Option<BlobRef> {
        self.blob
    }

    /// Returns the durable wall-clock instant.
    #[must_use]
    pub const fn occurred_at_ms(&self) -> u64 {
        self.occurred_at_ms
    }
}

/// Current durable run, cell, transaction, fence, and checkpoint state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSnapshot {
    run_id: RunId,
    project_id: ProjectId,
    transaction_id: WorkspaceTransactionId,
    cell_id: CellId,
    cell_revision: u64,
    fencing_token: FencingToken,
    lease_expires_at_ms: u64,
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
    baseline: Option<crate::CheckpointId>,
    result: Option<crate::CheckpointId>,
    environment: Option<Sha256Digest>,
    kernel_generation: Option<u64>,
    terminal_code: Option<String>,
    last_sequence: u64,
    revision: u64,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl RunSnapshot {
    /// Returns the immutable attempt identity.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Returns the stable project identity.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the workspace transaction identity.
    #[must_use]
    pub const fn transaction_id(&self) -> WorkspaceTransactionId {
        self.transaction_id
    }

    /// Returns the stable cell UUID.
    #[must_use]
    pub const fn cell_id(&self) -> CellId {
        self.cell_id
    }

    /// Returns the source revision for this stable cell.
    #[must_use]
    pub const fn cell_revision(&self) -> u64 {
        self.cell_revision
    }

    /// Returns the project writer fence captured by the attempt.
    #[must_use]
    pub const fn fencing_token(&self) -> FencingToken {
        self.fencing_token
    }

    /// Returns the durable lease expiration instant.
    #[must_use]
    pub const fn lease_expires_at_ms(&self) -> u64 {
        self.lease_expires_at_ms
    }

    /// Returns the durable run state.
    #[must_use]
    pub const fn run_state(&self) -> RunState {
        self.run_state
    }

    /// Returns the durable cell-attempt state.
    #[must_use]
    pub const fn cell_state(&self) -> CellState {
        self.cell_state
    }

    /// Returns the durable workspace transaction state.
    #[must_use]
    pub const fn transaction_state(&self) -> TransactionState {
        self.transaction_state
    }

    /// Returns the input checkpoint after begin completed.
    #[must_use]
    pub const fn baseline(&self) -> Option<crate::CheckpointId> {
        self.baseline
    }

    /// Returns the result checkpoint only after atomic success.
    #[must_use]
    pub const fn result(&self) -> Option<crate::CheckpointId> {
        self.result
    }

    /// Returns the durable environment fingerprint, never interpreter memory.
    #[must_use]
    pub const fn environment(&self) -> Option<Sha256Digest> {
        self.environment
    }

    /// Returns the worker generation reported for this attempt.
    #[must_use]
    pub const fn kernel_generation(&self) -> Option<u64> {
        self.kernel_generation
    }

    /// Returns the bounded stable terminal code when not successful.
    #[must_use]
    pub fn terminal_code(&self) -> Option<&str> {
        self.terminal_code.as_deref()
    }

    /// Returns the latest durable event sequence.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Returns the optimistic snapshot revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns when durable run intent was created.
    #[must_use]
    pub const fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Returns the latest durable update instant.
    #[must_use]
    pub const fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }
}

/// One bounded watch page resumable with `next_after_sequence`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchPage {
    events: Vec<RunEvent>,
    next_after_sequence: u64,
}

impl WatchPage {
    /// Returns ordered events with no sequence gaps introduced by this page.
    #[must_use]
    pub fn events(&self) -> &[RunEvent] {
        &self.events
    }

    /// Returns the cursor for the next watch call.
    #[must_use]
    pub const fn next_after_sequence(&self) -> u64 {
        self.next_after_sequence
    }
}

/// Public application error mapped by the `tactusd` transport adapter.
#[derive(Debug, Error)]
pub enum TactusError {
    /// A daemon resource or lifecycle setting was invalid.
    #[error("invalid Tactus configuration: {field}")]
    InvalidConfiguration {
        /// Invalid setting name.
        field: &'static str,
    },
    /// New commands are no longer accepted.
    #[error("Tactus daemon is draining or closed")]
    Closed,
    /// Active worker admission is at its configured capacity.
    #[error("active worker capacity is exhausted")]
    CapacityExhausted,
    /// Source bytes exceeded the configured request limit.
    #[error("cell source exceeds its byte limit")]
    SourceLimit,
    /// The same request ID was reused with another payload.
    #[error("idempotency key was reused with a different request")]
    IdempotencyKeyReused,
    /// Another unexpired writer owns the project.
    #[error("project lease is already held")]
    LeaseConflict,
    /// A stale or expired fence attempted a write.
    #[error("project fencing token was rejected")]
    FenceRejected,
    /// The run does not exist.
    #[error("run was not found")]
    NotFound,
    /// The command is invalid in the current state.
    #[error("run state does not allow this command")]
    InvalidTransition,
    /// Execute was rebound to another workspace path.
    #[error("workspace binding does not match begin")]
    WorkspaceBindingMismatch,
    /// Stored values failed schema/domain decoding.
    #[error("durable Tactus state is corrupt or incompatible")]
    DurableState,
    /// Canonical request hashing failed.
    #[error("canonical request digest failed")]
    Canonical(#[source] DigestError),
    /// Workspace/CAS checkpoint operation failed.
    #[error("workspace checkpoint operation failed")]
    Checkpoint(#[source] CheckpointError),
    /// Worker process or protocol operation failed before background acceptance.
    #[error("worker operation failed")]
    Worker(#[source] WorkerError),
    /// SQLite actor or migration infrastructure failed.
    #[error("Tactus durable storage failed")]
    Storage(#[source] Box<dyn Error + Send + Sync>),
    /// A named owned worker thread could not be created.
    #[error("failed to create owned worker thread")]
    ThreadSpawn(#[source] io::Error),
    /// Internal owner state was poisoned by an unexpected panic.
    #[error("Tactus owner state is unavailable")]
    OwnerState,
    /// Shutdown could not reap all workers before its deadline.
    #[error("Tactus shutdown deadline exceeded")]
    ShutdownDeadlineExceeded,
}

/// Owner of the Tactus repository, bounded worker set, and API composition.
pub struct TactusDaemon<W, B> {
    worker: Arc<W>,
    workspace: Arc<B>,
    repository: Repository,
    repository_owner: Option<RepositoryOwner>,
    clock: Arc<dyn Clock>,
    config: DaemonConfig,
    accepting: AtomicBool,
    active: Mutex<BTreeMap<RunId, ActiveRun>>,
}

struct ActiveRun {
    cancellation: Arc<CancellationSource>,
    join: JoinHandle<()>,
}

impl<W, B> TactusDaemon<W, B>
where
    W: WorkerPort + 'static,
    B: WorkspacePort + 'static,
{
    /// Opens the service-private database and reconciles all incomplete runs
    /// before enabling admission.
    ///
    /// # Errors
    ///
    /// Returns typed configuration, migration, integrity, or reconciliation
    /// failures. The caller must place `database_path` on an appropriate local
    /// state filesystem; project directories are not accepted implicitly.
    pub fn open(
        database_path: PathBuf,
        config: DaemonConfig,
        worker: W,
        workspace: B,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, TactusError> {
        let journal_mode = match config.durability {
            StateDurability::LocalWal => JournalMode::Wal,
            StateDurability::ConservativeDelete => JournalMode::Delete,
        };
        let owner = RepositoryOwner::open(
            database_path,
            256,
            Duration::from_secs(5),
            journal_mode,
            Duration::from_secs(10),
        )
        .map_err(map_repository_error)?;
        let repository = owner.repository();
        repository
            .reconcile_incomplete(clock.now_ms())
            .map_err(map_repository_error)?;
        Ok(Self {
            worker: Arc::new(worker),
            workspace: Arc::new(workspace),
            repository,
            repository_owner: Some(owner),
            clock,
            config,
            accepting: AtomicBool::new(true),
            active: Mutex::new(BTreeMap::new()),
        })
    }

    /// Persists intent and a fence, then publishes source and baseline CAS
    /// references before returning a pending run.
    ///
    /// # Errors
    ///
    /// Returns typed admission, idempotency, lease, checkpoint, or storage
    /// failures. A failed external publication terminalizes the written intent.
    pub fn begin(
        &self,
        request: BeginRequest,
        cancellation: &CancellationToken,
    ) -> Result<RunSnapshot, TactusError> {
        self.reap_finished()?;
        self.require_accepting()?;
        if request.source.len() > self.config.max_source_bytes {
            return Err(TactusError::SourceLimit);
        }
        if request.lease_ttl.is_zero() || request.lease_ttl > MAX_LEASE_TTL {
            return Err(TactusError::InvalidConfiguration {
                field: "begin lease TTL",
            });
        }
        let now = self.clock.now_ms();
        let expires_at = checked_deadline(now, request.lease_ttl)?;
        let workspace_binding =
            workspace_binding_digest(&request.workspace_root).map_err(TactusError::Checkpoint)?;
        let source = BlobRef::new(source_digest(&request.source), request.source.len() as u64);
        let request_digest = begin_request_digest(
            request.project_id,
            request.cell_id,
            source,
            workspace_binding,
        )?;
        let begun = self
            .repository
            .begin_intent(BeginIntent {
                request_id: request.request_id,
                request_digest,
                run_id: RunId::generate(),
                transaction_id: WorkspaceTransactionId::generate(),
                project_id: request.project_id,
                cell_id: request.cell_id,
                source,
                workspace_binding,
                owner_id: request.lease_owner_id,
                now_ms: now,
                expires_at_ms: expires_at,
            })
            .map_err(map_repository_error)?;
        if begun.replayed {
            return Ok(snapshot(begun.run));
        }

        let publication = (|| {
            if cancellation.is_cancelled() {
                return Err(CheckpointError::Cancelled);
            }
            let source_object = self.workspace.put_blob(&request.source)?;
            if source_object != source {
                return Err(CheckpointError::Integrity);
            }
            let baseline = self
                .workspace
                .capture(&request.workspace_root, cancellation)?;
            self.repository
                .activate(
                    begun.run.run_id,
                    begun.lease,
                    source_object,
                    &baseline,
                    self.clock.now_ms(),
                )
                .map_err(repository_as_checkpoint)?;
            Ok(())
        })();
        if let Err(error) = publication {
            let disposition = if matches!(error, CheckpointError::Cancelled) {
                FinishDisposition::Cancelled
            } else if matches!(error, CheckpointError::WorkspaceChanged) {
                FinishDisposition::Conflict
            } else {
                FinishDisposition::Failed
            };
            let code = match disposition {
                FinishDisposition::Cancelled => "CANCELLED",
                FinishDisposition::Conflict => "WORKSPACE_CHANGED",
                _ => "CHECKPOINT_FAILED",
            };
            let _ = self.repository.finish_terminal(
                begun.run.run_id,
                begun.lease,
                disposition,
                code,
                None,
                None,
                self.clock.now_ms(),
            );
            return Err(TactusError::Checkpoint(error));
        }
        self.status(begun.run.run_id)
    }

    /// Accepts execution, starts one owned named thread, and returns without
    /// waiting for the worker terminal result.
    ///
    /// # Errors
    ///
    /// Returns typed capacity, state, binding, thread, or storage failures.
    pub fn execute(&self, request: ExecuteRequest) -> Result<RunSnapshot, TactusError> {
        self.reap_finished()?;
        self.require_accepting()?;
        let mut active = self.active.lock().map_err(|_| TactusError::OwnerState)?;
        if active.len() >= self.config.max_active_runs {
            return Err(TactusError::CapacityExhausted);
        }
        if active.contains_key(&request.run_id) {
            return Err(TactusError::InvalidTransition);
        }
        let before = self
            .repository
            .run(request.run_id)
            .map_err(map_repository_error)?;
        if !before.source_is_published {
            return Err(TactusError::InvalidTransition);
        }
        let baseline_id = before.baseline.ok_or(TactusError::InvalidTransition)?;
        let binding =
            workspace_binding_digest(&request.workspace_root).map_err(TactusError::Checkpoint)?;
        if binding != before.workspace_binding {
            return Err(TactusError::WorkspaceBindingMismatch);
        }
        let baseline = self
            .repository
            .checkpoint(baseline_id)
            .map_err(map_repository_error)?;
        self.workspace
            .read_blob(baseline.manifest(), baseline.manifest().length())
            .map_err(TactusError::Checkpoint)?;
        let now = self.clock.now_ms();
        let execution_expiry = checked_deadline(now, self.config.execution_lease_ttl)?;
        let running = self
            .repository
            .start_execution(request.run_id, before.lease, binding, now, execution_expiry)
            .map_err(map_repository_error)?;
        let command = WorkerCommand::new(
            running.run_id,
            running.project_id,
            running.transaction_id,
            running.cell_id,
            running.source,
            baseline.id(),
            request.workspace_root,
        );
        let cancellation = Arc::new(CancellationSource::new());
        let thread_cancellation = Arc::clone(&cancellation);
        let repository = self.repository.clone();
        let workspace = Arc::clone(&self.workspace);
        let worker = Arc::clone(&self.worker);
        let clock = Arc::clone(&self.clock);
        let config = self.config;
        let run_for_thread = running.clone();
        let thread_name = format!("tactus-run-{}", running.run_id);
        let join = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute_background(
                        &repository,
                        workspace.as_ref(),
                        worker.as_ref(),
                        clock.as_ref(),
                        config,
                        run_for_thread.clone(),
                        command,
                        thread_cancellation.as_ref(),
                    )
                }));
                if result.is_err() {
                    let _ = repository.finish_terminal(
                        run_for_thread.run_id,
                        run_for_thread.lease,
                        FinishDisposition::Interrupted,
                        "RUNTIME_PANIC",
                        None,
                        None,
                        clock.now_ms(),
                    );
                }
            })
            .map_err(|source| {
                let _ = self.repository.finish_terminal(
                    running.run_id,
                    running.lease,
                    FinishDisposition::Interrupted,
                    "THREAD_SPAWN_FAILED",
                    None,
                    None,
                    self.clock.now_ms(),
                );
                TactusError::ThreadSpawn(source)
            })?;
        active.insert(running.run_id, ActiveRun { cancellation, join });
        Ok(snapshot(running))
    }

    /// Requests idempotent durable cancellation and signals the active worker
    /// owner when one exists.
    ///
    /// # Errors
    ///
    /// Returns typed lookup, fence, state, or storage failures.
    pub fn cancel(&self, run_id: RunId) -> Result<RunSnapshot, TactusError> {
        self.reap_finished()?;
        let (record, should_signal) = self
            .repository
            .request_cancel(run_id, self.clock.now_ms())
            .map_err(map_repository_error)?;
        if should_signal {
            let active = self.active.lock().map_err(|_| TactusError::OwnerState)?;
            if let Some(owner) = active.get(&run_id) {
                owner.cancellation.cancel();
            }
        }
        Ok(snapshot(record))
    }

    /// Returns a durable run snapshot. Worker or client memory is not queried.
    ///
    /// # Errors
    ///
    /// Returns typed lookup or storage failures.
    pub fn status(&self, run_id: RunId) -> Result<RunSnapshot, TactusError> {
        self.reap_finished()?;
        self.repository
            .run(run_id)
            .map(snapshot)
            .map_err(map_repository_error)
    }

    /// Returns a bounded durable event page after an explicit run-local cursor.
    ///
    /// # Errors
    ///
    /// Returns typed limit, lookup, compatibility, or storage failures.
    pub fn watch(
        &self,
        run_id: RunId,
        after_sequence: u64,
        limit: u32,
    ) -> Result<WatchPage, TactusError> {
        self.reap_finished()?;
        let stored = self
            .repository
            .watch(run_id, after_sequence, limit)
            .map_err(map_repository_error)?;
        let mut events = Vec::with_capacity(stored.len());
        for event in stored {
            events.push(run_event(event)?);
        }
        let next_after_sequence = events.last().map_or(after_sequence, RunEvent::sequence);
        Ok(WatchPage {
            events,
            next_after_sequence,
        })
    }

    /// Stops admission, cancels all workers, reaps every owned thread, then
    /// checkpoints and joins the SQLite writer.
    ///
    /// A deadline leaves ownership in this daemon so shutdown can be retried.
    ///
    /// # Errors
    ///
    /// Returns a typed deadline, owner-state, or storage failure.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), TactusError> {
        if timeout.is_zero() || timeout > MAX_LEASE_TTL {
            return Err(TactusError::InvalidConfiguration {
                field: "shutdown timeout",
            });
        }
        self.accepting.store(false, Ordering::Release);
        let deadline = Instant::now() + timeout;
        {
            let active = self.active.lock().map_err(|_| TactusError::OwnerState)?;
            for owner in active.values() {
                owner.cancellation.cancel();
            }
        }
        loop {
            self.reap_finished()?;
            if self
                .active
                .lock()
                .map_err(|_| TactusError::OwnerState)?
                .is_empty()
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(TactusError::ShutdownDeadlineExceeded);
            }
            thread::sleep(REAP_POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(TactusError::ShutdownDeadlineExceeded);
        }
        if let Some(owner) = self.repository_owner.as_mut() {
            owner.shutdown(remaining).map_err(map_repository_error)?;
        }
        self.repository_owner = None;
        Ok(())
    }

    fn require_accepting(&self) -> Result<(), TactusError> {
        if self.accepting.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(TactusError::Closed)
        }
    }

    fn reap_finished(&self) -> Result<(), TactusError> {
        let finished = {
            let mut active = self.active.lock().map_err(|_| TactusError::OwnerState)?;
            let ids: Vec<RunId> = active
                .iter()
                .filter_map(|(run_id, owner)| owner.join.is_finished().then_some(*run_id))
                .collect();
            ids.into_iter()
                .filter_map(|run_id| active.remove(&run_id))
                .collect::<Vec<_>>()
        };
        for owner in finished {
            owner.join.join().map_err(|_| TactusError::OwnerState)?;
        }
        Ok(())
    }
}

impl<W, B> Drop for TactusDaemon<W, B> {
    fn drop(&mut self) {
        self.accepting.store(false, Ordering::Release);
        if let Ok(active) = self.active.lock() {
            for owner in active.values() {
                owner.cancellation.cancel();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_background<W: WorkerPort, B: WorkspacePort>(
    repository: &Repository,
    workspace: &B,
    worker: &W,
    clock: &dyn Clock,
    config: DaemonConfig,
    run: RunRecord,
    command: WorkerCommand,
    cancellation: &CancellationSource,
) {
    let token = cancellation.token();
    let mut sink = DurableSink {
        repository,
        workspace,
        clock,
        run_id: run.run_id,
        lease: run.lease,
        cancellation,
        max_bytes: config.max_output_bytes,
        max_records: config.max_output_records,
        bytes: 0,
        records: 0,
    };
    let completion = worker.execute(command.clone(), &token, &mut sink);
    match completion {
        Ok(completion) => finish_worker_completion(
            repository,
            workspace,
            clock,
            run,
            command.workspace_root(),
            &token,
            completion,
        ),
        Err(error) => {
            let (disposition, code) = classify_worker_error(&error);
            let _ = repository.finish_terminal(
                run.run_id,
                run.lease,
                disposition,
                code,
                None,
                None,
                clock.now_ms(),
            );
        }
    }
}

fn finish_worker_completion<B: WorkspacePort>(
    repository: &Repository,
    workspace: &B,
    clock: &dyn Clock,
    run: RunRecord,
    workspace_root: &Path,
    cancellation: &CancellationToken,
    completion: WorkerCompletion,
) {
    match completion.terminal() {
        WorkerTerminal::Succeeded => match workspace.capture(workspace_root, cancellation) {
            Ok(result) => {
                let _ = repository.finish_success(
                    run.run_id,
                    run.lease,
                    &result,
                    completion.environment(),
                    completion.kernel_generation(),
                    clock.now_ms(),
                );
            }
            Err(error) => {
                let (disposition, code) = if matches!(error, CheckpointError::Cancelled) {
                    (FinishDisposition::Cancelled, "CANCELLED")
                } else if matches!(error, CheckpointError::WorkspaceChanged) {
                    (FinishDisposition::Conflict, "WORKSPACE_CHANGED")
                } else {
                    (FinishDisposition::Failed, "CHECKPOINT_FAILED")
                };
                let _ = repository.finish_terminal(
                    run.run_id,
                    run.lease,
                    disposition,
                    code,
                    Some(completion.environment()),
                    Some(completion.kernel_generation()),
                    clock.now_ms(),
                );
            }
        },
        WorkerTerminal::Failed(code) => {
            let _ = repository.finish_terminal(
                run.run_id,
                run.lease,
                FinishDisposition::Failed,
                code.as_str(),
                Some(completion.environment()),
                Some(completion.kernel_generation()),
                clock.now_ms(),
            );
        }
        WorkerTerminal::Cancelled => {
            let _ = repository.finish_terminal(
                run.run_id,
                run.lease,
                FinishDisposition::Cancelled,
                "CANCELLED",
                Some(completion.environment()),
                Some(completion.kernel_generation()),
                clock.now_ms(),
            );
        }
    }
}

fn classify_worker_error(error: &WorkerError) -> (FinishDisposition, &'static str) {
    match error {
        WorkerError::Cancelled => (FinishDisposition::Cancelled, "CANCELLED"),
        WorkerError::ProcessDied { .. } | WorkerError::MissingTerminal => {
            (FinishDisposition::Interrupted, "PROCESS_DIED")
        }
        WorkerError::DeadlineExceeded => (FinishDisposition::Failed, "DEADLINE_EXCEEDED"),
        WorkerError::ProcessOutputLimit | WorkerError::OutputChunkTooLarge => {
            (FinishDisposition::Failed, "OUTPUT_LIMIT")
        }
        WorkerError::SinkRejected => (FinishDisposition::Failed, "OUTPUT_PERSIST_FAILED"),
        _ => (FinishDisposition::Failed, "PROTOCOL_VIOLATION"),
    }
}

struct DurableSink<'a, B> {
    repository: &'a Repository,
    workspace: &'a B,
    clock: &'a dyn Clock,
    run_id: RunId,
    lease: LeaseGrant,
    cancellation: &'a CancellationSource,
    max_bytes: u64,
    max_records: u64,
    bytes: u64,
    records: u64,
}

impl<B: WorkspacePort> WorkerEventSink for DurableSink<'_, B> {
    fn output(
        &mut self,
        worker_sequence: u64,
        stream: OutputStream,
        bytes: &[u8],
    ) -> Result<(), WorkerError> {
        let length = u64::try_from(bytes.len()).map_err(|_| WorkerError::SinkRejected)?;
        let next_bytes = self
            .bytes
            .checked_add(length)
            .ok_or(WorkerError::SinkRejected)?;
        let next_records = self
            .records
            .checked_add(1)
            .ok_or(WorkerError::SinkRejected)?;
        if next_bytes > self.max_bytes || next_records > self.max_records {
            self.cancellation.cancel();
            return Err(WorkerError::SinkRejected);
        }
        let blob = self
            .workspace
            .put_blob(bytes)
            .map_err(|_| WorkerError::SinkRejected)?;
        self.repository
            .append_output(
                self.run_id,
                self.lease,
                worker_sequence,
                stream,
                blob,
                self.clock.now_ms(),
                self.max_bytes,
                self.max_records,
            )
            .map_err(|_| WorkerError::SinkRejected)?;
        self.bytes = next_bytes;
        self.records = next_records;
        Ok(())
    }
}

fn snapshot(record: RunRecord) -> RunSnapshot {
    RunSnapshot {
        run_id: record.run_id,
        project_id: record.project_id,
        transaction_id: record.transaction_id,
        cell_id: record.cell_id,
        cell_revision: record.cell_revision,
        fencing_token: record.lease.fence,
        lease_expires_at_ms: record.lease.expires_at_ms,
        run_state: record.state,
        cell_state: record.cell_state,
        transaction_state: record.transaction_state,
        baseline: record.baseline,
        result: record.result,
        environment: record.environment,
        kernel_generation: record.kernel_generation,
        terminal_code: record.terminal_code,
        last_sequence: record.last_sequence,
        revision: record.revision,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

fn run_event(event: StoredEvent) -> Result<RunEvent, TactusError> {
    Ok(RunEvent {
        sequence: event.sequence,
        kind: RunEventKind::parse(&event.kind).ok_or(TactusError::DurableState)?,
        worker_sequence: event.worker_sequence,
        stream: event.stream,
        blob: event.blob,
        occurred_at_ms: event.occurred_at_ms,
    })
}

fn source_digest(source: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(source).into())
}

fn begin_request_digest(
    project_id: ProjectId,
    cell_id: CellId,
    source: BlobRef,
    workspace_binding: Sha256Digest,
) -> Result<Sha256Digest, TactusError> {
    let mut hasher =
        CanonicalHasher::new("tactus.begin-request").map_err(TactusError::Canonical)?;
    hasher
        .write_field("cell_id", cell_id.to_string().as_bytes())
        .map_err(TactusError::Canonical)?;
    hasher
        .write_field("project_id", project_id.to_string().as_bytes())
        .map_err(TactusError::Canonical)?;
    hasher
        .write_field("source_digest", source.digest().as_bytes())
        .map_err(TactusError::Canonical)?;
    hasher
        .write_field("source_length", &source.length().to_be_bytes())
        .map_err(TactusError::Canonical)?;
    hasher
        .write_field("workspace_binding", workspace_binding.as_bytes())
        .map_err(TactusError::Canonical)?;
    Ok(hasher.finish())
}

fn checked_deadline(now_ms: u64, duration: Duration) -> Result<u64, TactusError> {
    let delta = u64::try_from(duration.as_millis())
        .map_err(|_| TactusError::InvalidConfiguration { field: "lease TTL" })?;
    now_ms
        .checked_add(delta)
        .ok_or(TactusError::InvalidConfiguration {
            field: "lease deadline",
        })
}

fn map_repository_error(error: RepositoryError) -> TactusError {
    match error {
        RepositoryError::NotFound => TactusError::NotFound,
        RepositoryError::IdempotencyConflict => TactusError::IdempotencyKeyReused,
        RepositoryError::LeaseConflict => TactusError::LeaseConflict,
        RepositoryError::FenceRejected => TactusError::FenceRejected,
        RepositoryError::InvalidTransition => TactusError::InvalidTransition,
        RepositoryError::WorkspaceBindingMismatch => TactusError::WorkspaceBindingMismatch,
        RepositoryError::CorruptState | RepositoryError::NumericOverflow => {
            TactusError::DurableState
        }
        other => TactusError::Storage(Box::new(other)),
    }
}

fn repository_as_checkpoint(error: RepositoryError) -> CheckpointError {
    match error {
        RepositoryError::FenceRejected => CheckpointError::WorkspaceChanged,
        _ => CheckpointError::Integrity,
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs};

    use tempfile::tempdir;

    use super::*;
    use crate::{
        Checkpoint, CheckpointBackendKind, CheckpointEntry, CheckpointId, RollbackFidelity,
    };

    #[derive(Clone, Default)]
    struct MemoryWorkspace {
        fail_reads: Arc<AtomicBool>,
    }

    impl MemoryWorkspace {
        fn fail_reads(&self) {
            self.fail_reads.store(true, Ordering::Release);
        }
    }

    impl WorkspacePort for MemoryWorkspace {
        fn put_blob(&self, bytes: &[u8]) -> Result<BlobRef, CheckpointError> {
            Ok(BlobRef::new(source_digest(bytes), bytes.len() as u64))
        }

        fn read_blob(&self, _object: BlobRef, _max_bytes: u64) -> Result<Vec<u8>, CheckpointError> {
            if self.fail_reads.load(Ordering::Acquire) {
                Err(CheckpointError::Cas)
            } else {
                Ok(Vec::new())
            }
        }

        fn capture(
            &self,
            workspace_root: &Path,
            cancellation: &CancellationToken,
        ) -> Result<Checkpoint, CheckpointError> {
            if cancellation.is_cancelled() {
                return Err(CheckpointError::Cancelled);
            }
            let digest = workspace_binding_digest(workspace_root)?;
            Ok(Checkpoint::from_stored(
                CheckpointId::from_digest(digest),
                BlobRef::new(digest, 0),
                CheckpointBackendKind::NonGit,
                RollbackFidelity::FullManifest,
                None,
                Vec::<CheckpointEntry>::new(),
                0,
            ))
        }

        fn restore_declared(
            &self,
            _workspace_root: &Path,
            _baseline: &Checkpoint,
            _observed: &Checkpoint,
            _declared_paths: &[String],
            _cancellation: &CancellationToken,
        ) -> Result<crate::RestoreReport, CheckpointError> {
            Err(CheckpointError::InvalidConfiguration { field: "not used" })
        }
    }

    struct SuccessWorker;

    impl WorkerPort for SuccessWorker {
        fn execute(
            &self,
            _command: WorkerCommand,
            _cancellation: &CancellationToken,
            sink: &mut dyn WorkerEventSink,
        ) -> Result<WorkerCompletion, WorkerError> {
            sink.output(3, OutputStream::Stdout, b"hello")?;
            WorkerCompletion::new(
                Sha256Digest::from_bytes([4; 32]),
                1,
                WorkerTerminal::Succeeded,
            )
        }
    }

    struct BlockingWorker;

    impl WorkerPort for BlockingWorker {
        fn execute(
            &self,
            _command: WorkerCommand,
            cancellation: &CancellationToken,
            _sink: &mut dyn WorkerEventSink,
        ) -> Result<WorkerCompletion, WorkerError> {
            while !cancellation.is_cancelled() {
                thread::yield_now();
            }
            Err(WorkerError::Cancelled)
        }
    }

    #[test]
    fn begin_execute_status_watch_form_a_durable_vertical_slice() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        let state = temporary.path().join("state");
        fs::create_dir(&workspace)?;
        fs::create_dir(&state)?;
        let clock = Arc::new(ManualClock::new(1_000));
        let mut daemon = TactusDaemon::open(
            state.join("tactus.db"),
            DaemonConfig::default(),
            SuccessWorker,
            MemoryWorkspace::default(),
            clock,
        )?;
        let begun = daemon.begin(
            BeginRequest::new(
                RequestId::generate(),
                ProjectId::generate(),
                CellId::generate(),
                b"print('hello')\n".to_vec(),
                workspace.clone(),
                LeaseOwnerId::generate(),
                Duration::from_secs(60),
            ),
            &CancellationToken::new(),
        )?;
        assert_eq!(begun.run_state(), RunState::Pending);
        daemon.execute(ExecuteRequest::new(begun.run_id(), workspace))?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let terminal = loop {
            let current = daemon.status(begun.run_id())?;
            if current.run_state().is_terminal() {
                break current;
            }
            if Instant::now() >= deadline {
                return Err("run did not become terminal".into());
            }
            thread::yield_now();
        };
        assert_eq!(terminal.run_state(), RunState::Succeeded);
        assert!(terminal.result().is_some());
        let page = daemon.watch(begun.run_id(), 0, 100)?;
        assert!(
            page.events()
                .iter()
                .any(|event| event.kind() == RunEventKind::Output)
        );
        assert_eq!(
            page.events().last().map(RunEvent::kind),
            Some(RunEventKind::Succeeded)
        );
        assert_eq!(
            daemon.cancel(begun.run_id())?.run_state(),
            RunState::Succeeded
        );
        daemon.shutdown(Duration::from_secs(2))?;
        Ok(())
    }

    #[test]
    fn idempotent_begin_replays_same_run_and_rejects_changed_payload() -> Result<(), Box<dyn Error>>
    {
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        let state = temporary.path().join("state");
        fs::create_dir(&workspace)?;
        fs::create_dir(&state)?;
        let mut daemon = TactusDaemon::open(
            state.join("tactus.db"),
            DaemonConfig::default(),
            SuccessWorker,
            MemoryWorkspace::default(),
            Arc::new(ManualClock::new(1_000)),
        )?;
        let request_id = RequestId::generate();
        let project_id = ProjectId::generate();
        let cell_id = CellId::generate();
        let owner_id = LeaseOwnerId::generate();
        let request = |source: &[u8]| {
            BeginRequest::new(
                request_id,
                project_id,
                cell_id,
                source.to_vec(),
                workspace.clone(),
                owner_id,
                Duration::from_secs(60),
            )
        };
        let first = daemon.begin(request(b"one"), &CancellationToken::new())?;
        let replay = daemon.begin(request(b"one"), &CancellationToken::new())?;
        assert_eq!(first.run_id(), replay.run_id());
        assert!(matches!(
            daemon.begin(request(b"two"), &CancellationToken::new()),
            Err(TactusError::IdempotencyKeyReused)
        ));
        daemon.cancel(first.run_id())?;
        daemon.shutdown(Duration::from_secs(2))?;
        Ok(())
    }

    #[test]
    fn baseline_lookup_failure_does_not_start_execution_or_add_owner() -> Result<(), Box<dyn Error>>
    {
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        let state = temporary.path().join("state");
        fs::create_dir(&workspace)?;
        fs::create_dir(&state)?;
        let fake_workspace = MemoryWorkspace::default();
        let mut daemon = TactusDaemon::open(
            state.join("tactus.db"),
            DaemonConfig::default(),
            SuccessWorker,
            fake_workspace.clone(),
            Arc::new(ManualClock::new(1_000)),
        )?;
        let begun = daemon.begin(
            BeginRequest::new(
                RequestId::generate(),
                ProjectId::generate(),
                CellId::generate(),
                b"print('hello')\n".to_vec(),
                workspace.clone(),
                LeaseOwnerId::generate(),
                Duration::from_secs(60),
            ),
            &CancellationToken::new(),
        )?;
        assert!(begun.baseline().is_some());
        let owners_before = daemon
            .active
            .lock()
            .map_err(|_| TactusError::OwnerState)?
            .len();

        fake_workspace.fail_reads();
        let failure = daemon.execute(ExecuteRequest::new(begun.run_id(), workspace));
        let after = daemon.status(begun.run_id())?;
        let owners_after = daemon
            .active
            .lock()
            .map_err(|_| TactusError::OwnerState)?
            .len();

        daemon.cancel(begun.run_id())?;
        daemon.shutdown(Duration::from_secs(2))?;

        assert!(matches!(
            failure,
            Err(TactusError::Checkpoint(CheckpointError::Cas))
        ));
        assert_eq!(after.run_state(), RunState::Pending);
        assert_eq!(after.cell_state(), CellState::Queued);
        assert_eq!(after.transaction_state(), TransactionState::Active);
        assert_eq!(owners_after, owners_before);
        Ok(())
    }

    #[test]
    fn cancel_signals_owned_worker_and_commits_cancelled_state() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        let state = temporary.path().join("state");
        fs::create_dir(&workspace)?;
        fs::create_dir(&state)?;
        let mut daemon = TactusDaemon::open(
            state.join("tactus.db"),
            DaemonConfig::default(),
            BlockingWorker,
            MemoryWorkspace::default(),
            Arc::new(ManualClock::new(1_000)),
        )?;
        let begun = daemon.begin(
            BeginRequest::new(
                RequestId::generate(),
                ProjectId::generate(),
                CellId::generate(),
                b"while True: pass\n".to_vec(),
                workspace.clone(),
                LeaseOwnerId::generate(),
                Duration::from_secs(60),
            ),
            &CancellationToken::new(),
        )?;
        daemon.execute(ExecuteRequest::new(begun.run_id(), workspace))?;
        let cancelling = daemon.cancel(begun.run_id())?;
        assert!(matches!(
            cancelling.run_state(),
            RunState::Cancelling | RunState::Cancelled
        ));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let current = daemon.status(begun.run_id())?;
            if current.run_state() == RunState::Cancelled {
                break;
            }
            if Instant::now() >= deadline {
                return Err("cancel did not become durable".into());
            }
            thread::yield_now();
        }
        daemon.shutdown(Duration::from_secs(2))?;
        Ok(())
    }
}
