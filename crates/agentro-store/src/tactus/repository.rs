use std::{path::PathBuf, time::Duration};

use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};
use thiserror::Error;

use crate::{
    MigrationDefinitionError, MigrationProfile, StoreActor, StoreConfig, StoreError, StoreHandle,
};

use super::{
    codec::{
        CorruptStorageError, decode_boolean, decode_cell_key, decode_checkpoint_key, decode_digest,
        decode_lease_owner_key, decode_non_negative, decode_optional_blob, decode_positive,
        decode_project_key, decode_run_key, decode_transaction_key, encode_integer,
    },
    migration::tactus_v1_migration,
    model::{
        AppendOutput, BeginIntent, BeginIntentResult, BlobRef, CellState, CheckpointBackend,
        CheckpointEntry, CheckpointEntryKind, CheckpointKey, CheckpointRecord, FencingToken,
        FinishDisposition, FinishSuccess, FinishTerminal, LeaseGrant, OutputStream,
        RollbackFidelity, RunKey, RunRecord, RunState, StoredEvent, TransactionState,
    },
};

const TACTUS_SCHEMA_VERSION: u32 = 1;
const STORE_REPLY_TIMEOUT: Duration = Duration::from_secs(10);
type ActivationRow = (String, String, i64, String, i64, String, String, i64);

/// Frozen upper bound for one later Tactus event page.
pub const MAX_WATCH_EVENTS: u32 = 1_000;

/// Maximum incomplete runs supported by one startup reconciliation.
///
/// Reconciliation is intentionally all-or-nothing at the current product
/// scale. Exceeding this bound fails without changing any selected run.
pub const MAX_RECONCILE_RUNS: u32 = 1_000;

/// Failures while opening, checking, or shutting down Tactus storage.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RepositoryError {
    /// Shared SQLite actor, migration, or lifecycle failure.
    #[error("Tactus SQLite owner failed")]
    Store(#[from] StoreError),
    /// The frozen migration definition violated shared migration bounds.
    #[error("Tactus v1 migration definition is invalid")]
    Migration(#[from] MigrationDefinitionError),
    /// A live handle did not report the only supported Tactus schema version.
    #[error("unexpected Tactus schema version {found}; expected {expected}")]
    UnexpectedSchemaVersion {
        /// Version read from the migration ledger.
        found: u32,
        /// Version supported by this repository owner.
        expected: u32,
    },
    /// No durable run exists for the supplied identity.
    #[error("run was not found")]
    NotFound,
    /// A request identity was reused with different content.
    #[error("request ID was reused with a different payload")]
    IdempotencyConflict,
    /// The project already has a lease whose expiry is after the supplied time.
    #[error("project already has a live lease")]
    LeaseConflict,
    /// The supplied project lease is stale, mismatched, or expired.
    #[error("project fencing token is stale or expired")]
    FenceRejected,
    /// The durable run or workspace transaction state rejects the operation.
    #[error("run state does not allow the requested operation")]
    InvalidTransition,
    /// The supplied workspace binding differs from the durable run binding.
    #[error("workspace binding does not match the durable run")]
    WorkspaceBindingMismatch,
    /// A persisted Tactus value or cross-column invariant is invalid.
    #[error("durable Tactus state is corrupt or from an unsupported schema")]
    CorruptState,
    /// An unsigned value or checked durable counter exceeds SQLite's range.
    #[error("durable integer value is outside SQLite range")]
    NumericOverflow,
    /// Appending one output record would exceed its durable run budget.
    #[error("durable output budget would be exceeded")]
    OutputBudgetExceeded,
    /// Startup found more incomplete runs than the current all-or-nothing bound.
    #[error("incomplete run count exceeds reconciliation limit {limit}")]
    ReconciliationLimitExceeded {
        /// Maximum incomplete runs accepted by one reconciliation.
        limit: u32,
    },
}

/// Unique owner of the Tactus SQLite writer actor.
pub struct RepositoryOwner {
    actor: StoreActor,
    repository: Repository,
}

impl RepositoryOwner {
    /// Opens Tactus storage and applies or verifies the frozen v1 schema.
    ///
    /// The Tactus three-column ledger and checksum profile is selected here,
    /// rather than delegated to a caller or changing the shared default.
    ///
    /// # Errors
    ///
    /// Returns typed migration, actor startup, SQLite, or path errors.
    pub fn open(
        database: PathBuf,
        config: StoreConfig,
        startup_timeout: Duration,
    ) -> Result<Self, RepositoryError> {
        let migration = tactus_v1_migration()?;
        let actor = StoreActor::start_with_migration_profile(
            database,
            config,
            vec![migration],
            MigrationProfile::TactusV1Compatibility,
            startup_timeout,
        )?;
        let repository = Repository {
            handle: actor.handle(),
        };
        Ok(Self { actor, repository })
    }

    /// Returns a cloneable typed repository handle without shutdown authority.
    #[must_use]
    pub fn repository(&self) -> Repository {
        self.repository.clone()
    }

    /// Closes admission, checkpoints WAL state, and joins the writer.
    ///
    /// # Errors
    ///
    /// Returns typed actor shutdown or SQLite errors.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), RepositoryError> {
        self.actor.shutdown(timeout).map_err(RepositoryError::Store)
    }
}

/// Cloneable Tactus storage handle exposing only typed repository operations.
#[derive(Clone)]
pub struct Repository {
    handle: StoreHandle,
}

impl Repository {
    /// Reads the current Tactus migration version through the writer actor.
    ///
    /// # Errors
    ///
    /// Returns typed actor admission, deadline, or SQLite errors.
    pub fn schema_version(&self, reply_timeout: Duration) -> Result<u32, RepositoryError> {
        self.handle
            .schema_version(reply_timeout)
            .map_err(RepositoryError::Store)
    }

    /// Verifies that the repository is ready at the frozen Tactus v1 schema.
    ///
    /// # Errors
    ///
    /// Returns actor errors or [`RepositoryError::UnexpectedSchemaVersion`].
    pub fn ensure_schema_ready(&self, reply_timeout: Duration) -> Result<(), RepositoryError> {
        let found = self.schema_version(reply_timeout)?;
        if found == TACTUS_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(RepositoryError::UnexpectedSchemaVersion {
                found,
                expected: TACTUS_SCHEMA_VERSION,
            })
        }
    }

    /// Creates a fenced pending run or replays the run for an existing request.
    ///
    /// Request replay is checked before lease admission. A newly admitted
    /// intent advances the project fence and cell revision, then writes the
    /// run, prepared workspace transaction, and first event atomically.
    ///
    /// # Errors
    ///
    /// Returns typed idempotency, lease, overflow, corruption, actor, or
    /// SQLite failures.
    pub fn begin_intent(&self, input: BeginIntent) -> Result<BeginIntentResult, RepositoryError> {
        let request_id = input.request_id.to_string();
        let request_digest = input.request_digest.to_string();
        let run_id = input.run_id.to_string();
        let transaction_id = input.transaction_id.to_string();
        let project_id = input.project_id.to_string();
        let cell_id = input.cell_id.to_string();
        let owner_id = input.owner_id.to_string();
        let source_digest = input.source.digest.to_string();
        let source_length = sqlite_integer("source_length", input.source.length)?;
        let workspace_binding = input.workspace_binding.to_string();
        let now_ms = sqlite_integer("now_ms", input.now_ms)?;
        let expires_at_ms = sqlite_integer("expires_at_ms", input.expires_at_ms)?;
        let generated_run = input.run_id;

        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let existing: Option<(String, String)> = transaction
                .query_row(
                    "SELECT request_digest, run_id FROM runs WHERE request_id = ?1",
                    [&request_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((existing_digest, existing_run)) = existing {
                return Ok(if existing_digest == request_digest {
                    BeginDbOutcome::Replay(existing_run)
                } else {
                    BeginDbOutcome::IdempotencyConflict
                });
            }

            transaction.execute(
                "INSERT INTO projects (project_id, last_fence) VALUES (?1, 0)
                     ON CONFLICT(project_id) DO NOTHING",
                [&project_id],
            )?;
            let live_lease: Option<(String, i64, i64)> = transaction
                .query_row(
                    "SELECT owner_id, fence, expires_at_ms
                     FROM project_leases WHERE project_id = ?1",
                    [&project_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if let Some((lease_owner, lease_fence, lease_expiry)) = live_lease {
                if decode_lease_owner_key("owner_id", &lease_owner).is_err()
                    || decode_positive("fence", lease_fence).is_err()
                    || decode_non_negative("expires_at_ms", lease_expiry).is_err()
                {
                    return Ok(BeginDbOutcome::CorruptState);
                }
                if lease_expiry > now_ms {
                    return Ok(BeginDbOutcome::LeaseConflict);
                }
            }

            let last_fence: i64 = transaction.query_row(
                "SELECT last_fence FROM projects WHERE project_id = ?1",
                [&project_id],
                |row| row.get(0),
            )?;
            if last_fence < 0 {
                return Ok(BeginDbOutcome::CorruptState);
            }
            let Some(fence) = last_fence.checked_add(1) else {
                return Ok(BeginDbOutcome::NumericOverflow);
            };
            if transaction.execute(
                "UPDATE projects SET last_fence = ?2 WHERE project_id = ?1",
                params![project_id, fence],
            )? != 1
            {
                return Ok(BeginDbOutcome::CorruptState);
            }
            transaction.execute(
                "INSERT INTO project_leases (project_id, owner_id, fence, expires_at_ms)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(project_id) DO UPDATE SET
                         owner_id = excluded.owner_id,
                         fence = excluded.fence,
                         expires_at_ms = excluded.expires_at_ms",
                params![project_id, owner_id, fence, expires_at_ms],
            )?;

            let cell: Option<(i64, String)> = transaction
                .query_row(
                    "SELECT revision, source_digest FROM cells WHERE cell_id = ?1",
                    [&cell_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let cell_revision = match cell {
                None => {
                    transaction.execute(
                        "INSERT INTO cells
                             (cell_id, revision, source_digest, created_at_ms, updated_at_ms)
                             VALUES (?1, 1, ?2, ?3, ?3)",
                        params![cell_id, source_digest, now_ms],
                    )?;
                    1_i64
                }
                Some((revision, _)) if revision <= 0 => {
                    return Ok(BeginDbOutcome::CorruptState);
                }
                Some((revision, existing_digest)) if existing_digest == source_digest => revision,
                Some((revision, _)) => {
                    let Some(next_revision) = revision.checked_add(1) else {
                        return Ok(BeginDbOutcome::NumericOverflow);
                    };
                    if transaction.execute(
                        "UPDATE cells
                             SET revision = ?2, source_digest = ?3, updated_at_ms = ?4
                             WHERE cell_id = ?1",
                        params![cell_id, next_revision, source_digest, now_ms],
                    )? != 1
                    {
                        return Ok(BeginDbOutcome::CorruptState);
                    }
                    next_revision
                }
            };
            transaction.execute(
                "INSERT INTO runs (
                        run_id, request_id, request_digest, project_id, transaction_id,
                        cell_id, cell_revision, lease_owner_id, fence, lease_expires_at_ms,
                        workspace_binding_digest, state, cell_state, source_digest,
                        source_length, last_sequence, revision, created_at_ms, updated_at_ms
                     ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                        ?11, 'pending', 'queued', ?12, ?13, 1, 1, ?14, ?14
                     )",
                params![
                    run_id,
                    request_id,
                    request_digest,
                    project_id,
                    transaction_id,
                    cell_id,
                    cell_revision,
                    owner_id,
                    fence,
                    expires_at_ms,
                    workspace_binding,
                    source_digest,
                    source_length,
                    now_ms,
                ],
            )?;
            transaction.execute(
                "INSERT INTO workspace_transactions (
                        transaction_id, run_id, project_id, fence, state,
                        created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, 'prepared', ?5, ?5)",
                params![transaction_id, run_id, project_id, fence, now_ms],
            )?;
            transaction.execute(
                "INSERT INTO events (run_id, sequence, kind, occurred_at_ms)
                     VALUES (?1, 1, 'intent_created', ?2)",
                params![run_id, now_ms],
            )?;
            transaction.commit()?;
            Ok(BeginDbOutcome::Created)
        })?;

        let (run_id, replayed) = match outcome {
            BeginDbOutcome::Created => (generated_run, false),
            BeginDbOutcome::Replay(existing) => {
                (decode_storage(decode_run_key("run_id", &existing))?, true)
            }
            BeginDbOutcome::IdempotencyConflict => {
                return Err(RepositoryError::IdempotencyConflict);
            }
            BeginDbOutcome::LeaseConflict => return Err(RepositoryError::LeaseConflict),
            BeginDbOutcome::CorruptState => return Err(RepositoryError::CorruptState),
            BeginDbOutcome::NumericOverflow => return Err(RepositoryError::NumericOverflow),
        };
        let run = self.run(run_id)?;
        Ok(BeginIntentResult {
            lease: run.lease,
            run,
            replayed,
        })
    }

    /// Publishes source and baseline checkpoint metadata for a pending intent.
    ///
    /// The run remains pending while its workspace transaction atomically
    /// advances from prepared to active and receives a `workspace_ready` event.
    ///
    /// # Errors
    ///
    /// Returns typed fence, state, overflow, corruption, actor, or SQLite
    /// failures.
    pub fn activate(
        &self,
        run_id: RunKey,
        lease: LeaseGrant,
        source: BlobRef,
        baseline: &CheckpointRecord,
        now_ms: u64,
    ) -> Result<RunRecord, RepositoryError> {
        let run_text = run_id.to_string();
        let source_digest = source.digest.to_string();
        let source_length = sqlite_integer("source_length", source.length)?;
        let now = sqlite_integer("now_ms", now_ms)?;
        let lease = PreparedLease::new(lease)?;
        let checkpoint = PreparedCheckpoint::new(baseline)?;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            match check_fence(&transaction, &lease, now)? {
                FenceStatus::Current => {}
                FenceStatus::Rejected => return Ok(MutationOutcome::FenceRejected),
                FenceStatus::Corrupt => return Ok(MutationOutcome::CorruptState),
            }
            let current: Option<ActivationRow> = transaction
                .query_row(
                    "SELECT state, source_digest, source_length, transaction_id, revision,
                            project_id, lease_owner_id, fence
                         FROM runs WHERE run_id = ?1",
                    [&run_text],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                state,
                expected_digest,
                expected_length,
                transaction_id,
                revision,
                project_id,
                owner_id,
                fence,
            )) = current
            else {
                return Ok(MutationOutcome::NotFound);
            };
            let lease_matches = match lease.matches_stored(&project_id, &owner_id, fence) {
                Ok(matches) => matches,
                Err(_) => return Ok(MutationOutcome::CorruptState),
            };
            if !lease_matches {
                return Ok(MutationOutcome::FenceRejected);
            }
            if state != RunState::Pending.as_str()
                || expected_digest != source_digest
                || expected_length != source_length
            {
                return Ok(MutationOutcome::InvalidTransition);
            }
            let Some(next_revision) = checked_revision(revision) else {
                return Ok(counter_failure(revision));
            };

            if !register_checkpoint(&transaction, &checkpoint, now)? {
                return Ok(MutationOutcome::CorruptState);
            }
            let changed = transaction.execute(
                "UPDATE runs
                     SET source_object_digest = ?2, baseline_checkpoint_id = ?3,
                         revision = ?4, updated_at_ms = ?5
                     WHERE run_id = ?1 AND state = 'pending'",
                params![run_text, source_digest, checkpoint.id, next_revision, now],
            )?;
            let transaction_changed = transaction.execute(
                "UPDATE workspace_transactions
                     SET state = 'active', baseline_checkpoint_id = ?2, updated_at_ms = ?3
                     WHERE transaction_id = ?1 AND state = 'prepared'",
                params![transaction_id, checkpoint.id, now],
            )?;
            if changed != 1 || transaction_changed != 1 {
                return Ok(MutationOutcome::InvalidTransition);
            }
            match append_lifecycle_event(&transaction, &run_text, "workspace_ready", now)? {
                EventAppendOutcome::Applied(_) => {}
                EventAppendOutcome::CorruptState => {
                    return Ok(MutationOutcome::CorruptState);
                }
                EventAppendOutcome::NumericOverflow => {
                    return Ok(MutationOutcome::NumericOverflow);
                }
            }
            transaction.commit()?;
            Ok(MutationOutcome::Applied)
        })?;
        map_mutation(outcome)?;
        self.run(run_id)
    }

    /// Starts a pending run from an active workspace transaction.
    ///
    /// The current project lease is extended in the same immediate transaction
    /// that advances run/cell state and appends the `running` event.
    ///
    /// # Errors
    ///
    /// Returns typed fence, binding, state, overflow, corruption, actor, or
    /// SQLite failures.
    pub fn start_execution(
        &self,
        run_id: RunKey,
        lease: LeaseGrant,
        workspace_binding: agentro_contracts::Sha256Digest,
        now_ms: u64,
        execution_expires_at_ms: u64,
    ) -> Result<RunRecord, RepositoryError> {
        let run_text = run_id.to_string();
        let binding = workspace_binding.to_string();
        let now = sqlite_integer("now_ms", now_ms)?;
        let new_expiry = sqlite_integer("execution_expires_at_ms", execution_expires_at_ms)?;
        let lease = PreparedLease::new(lease)?;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            match check_fence(&transaction, &lease, now)? {
                FenceStatus::Current => {}
                FenceStatus::Rejected => return Ok(MutationOutcome::FenceRejected),
                FenceStatus::Corrupt => return Ok(MutationOutcome::CorruptState),
            }
            let current: Option<(String, String, String, i64, String, String, i64)> = transaction
                .query_row(
                    "SELECT r.state, r.workspace_binding_digest, w.state, r.revision,
                            r.project_id, r.lease_owner_id, r.fence
                         FROM runs r
                         JOIN workspace_transactions w ON w.run_id = r.run_id
                         WHERE r.run_id = ?1",
                    [&run_text],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                state,
                stored_binding,
                transaction_state,
                revision,
                project_id,
                owner_id,
                fence,
            )) = current
            else {
                return Ok(MutationOutcome::NotFound);
            };
            let lease_matches = match lease.matches_stored(&project_id, &owner_id, fence) {
                Ok(matches) => matches,
                Err(_) => return Ok(MutationOutcome::CorruptState),
            };
            if !lease_matches {
                return Ok(MutationOutcome::FenceRejected);
            }
            if stored_binding != binding {
                return Ok(MutationOutcome::WorkspaceBindingMismatch);
            }
            if state != RunState::Pending.as_str()
                || transaction_state != TransactionState::Active.as_str()
            {
                return Ok(MutationOutcome::InvalidTransition);
            }
            let Some(next_revision) = checked_revision(revision) else {
                return Ok(counter_failure(revision));
            };

            if transaction.execute(
                "UPDATE project_leases SET expires_at_ms = ?2
                     WHERE project_id = ?1 AND owner_id = ?3 AND fence = ?4",
                params![lease.project_id, new_expiry, lease.owner_id, lease.fence],
            )? != 1
            {
                return Ok(MutationOutcome::FenceRejected);
            }
            if transaction.execute(
                "UPDATE runs
                     SET state = 'running', cell_state = 'running',
                         lease_expires_at_ms = ?2, revision = ?3,
                         updated_at_ms = ?4
                     WHERE run_id = ?1 AND state = 'pending'",
                params![run_text, new_expiry, next_revision, now],
            )? != 1
            {
                return Ok(MutationOutcome::InvalidTransition);
            }
            match append_lifecycle_event(&transaction, &run_text, "running", now)? {
                EventAppendOutcome::Applied(_) => {}
                EventAppendOutcome::CorruptState => {
                    return Ok(MutationOutcome::CorruptState);
                }
                EventAppendOutcome::NumericOverflow => {
                    return Ok(MutationOutcome::NumericOverflow);
                }
            }
            transaction.commit()?;
            Ok(MutationOutcome::Applied)
        })?;
        map_mutation(outcome)?;
        self.run(run_id)
    }

    /// Atomically registers one immutable worker output reference and event.
    ///
    /// A repeated worker sequence returns its original run-local event sequence
    /// only when stream, digest, and length match. Budget accounting excludes
    /// such replays and is based on durable output rows, not caller memory.
    ///
    /// # Errors
    ///
    /// Returns typed fence, state, replay-conflict, budget, overflow,
    /// corruption, actor, or SQLite failures.
    pub fn append_output(&self, input: AppendOutput) -> Result<u64, RepositoryError> {
        if input.worker_sequence == 0 {
            return Err(RepositoryError::InvalidTransition);
        }
        let run_text = input.run_id.to_string();
        let worker_sequence = sqlite_integer("worker_sequence", input.worker_sequence)?;
        let stream_text = input.stream.as_str();
        let blob_digest = input.blob.digest.to_string();
        let blob_length = sqlite_integer("blob_length", input.blob.length)?;
        let now = sqlite_integer("now_ms", input.now_ms)?;
        let max_bytes = sqlite_integer("max_output_bytes", input.budget.max_bytes())?;
        let max_records = sqlite_integer("max_output_records", input.budget.max_records())?;
        let lease = PreparedLease::new(input.lease)?;
        let stream = input.stream;
        let blob = input.blob;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            match check_fence(&transaction, &lease, now)? {
                FenceStatus::Current => {}
                FenceStatus::Rejected => return Ok(OutputMutation::FenceRejected),
                FenceStatus::Corrupt => return Ok(OutputMutation::CorruptState),
            }
            let current: Option<(String, String, String, i64)> = transaction
                .query_row(
                    "SELECT state, project_id, lease_owner_id, fence
                     FROM runs WHERE run_id = ?1",
                    [&run_text],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((state, project_id, owner_id, fence)) = current else {
                return Ok(OutputMutation::NotFound);
            };
            let lease_matches = match lease.matches_stored(&project_id, &owner_id, fence) {
                Ok(matches) => matches,
                Err(_) => return Ok(OutputMutation::CorruptState),
            };
            if !lease_matches {
                return Ok(OutputMutation::FenceRejected);
            }
            let state = match RunState::decode("state", &state) {
                Ok(state) => state,
                Err(_) => return Ok(OutputMutation::CorruptState),
            };
            if !matches!(state, RunState::Running | RunState::Cancelling) {
                return Ok(OutputMutation::InvalidTransition);
            }

            let existing: Option<RawOutputReplay> = transaction
                .query_row(
                    "SELECT e.sequence, e.kind, e.stream, e.blob_digest, e.blob_length,
                            o.stream, o.blob_digest, o.blob_length
                     FROM events e
                     LEFT JOIN output_chunks o
                       ON o.run_id = e.run_id AND o.event_sequence = e.sequence
                     WHERE e.run_id = ?1 AND e.worker_sequence = ?2",
                    params![run_text, worker_sequence],
                    map_raw_output_replay,
                )
                .optional()?;
            if let Some(existing) = existing {
                return Ok(existing.compare(stream, blob));
            }

            let (record_count, byte_count): (i64, i64) = transaction.query_row(
                "SELECT COUNT(event_sequence), COALESCE(SUM(blob_length), 0)
                 FROM output_chunks WHERE run_id = ?1",
                [&run_text],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if record_count < 0 || byte_count < 0 {
                return Ok(OutputMutation::CorruptState);
            }
            let Some(next_records) = record_count.checked_add(1) else {
                return Ok(OutputMutation::NumericOverflow);
            };
            let Some(next_bytes) = byte_count.checked_add(blob_length) else {
                return Ok(OutputMutation::NumericOverflow);
            };
            if next_records > max_records || next_bytes > max_bytes {
                return Ok(OutputMutation::BudgetExceeded);
            }

            let event = EventData::output(worker_sequence, stream_text, &blob_digest, blob_length);
            let sequence = match append_event(&transaction, &run_text, event, now)? {
                EventAppendOutcome::Applied(sequence) => sequence,
                EventAppendOutcome::CorruptState => {
                    return Ok(OutputMutation::CorruptState);
                }
                EventAppendOutcome::NumericOverflow => {
                    return Ok(OutputMutation::NumericOverflow);
                }
            };
            transaction.execute(
                "INSERT INTO output_chunks
                     (run_id, event_sequence, stream, blob_digest, blob_length)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![run_text, sequence, stream_text, blob_digest, blob_length],
            )?;
            transaction.commit()?;
            Ok(OutputMutation::Applied(sequence))
        })?;
        match outcome {
            OutputMutation::Applied(sequence) => {
                decode_storage(decode_positive("sequence", sequence))
            }
            OutputMutation::FenceRejected => Err(RepositoryError::FenceRejected),
            OutputMutation::NotFound => Err(RepositoryError::NotFound),
            OutputMutation::InvalidTransition => Err(RepositoryError::InvalidTransition),
            OutputMutation::BudgetExceeded => Err(RepositoryError::OutputBudgetExceeded),
            OutputMutation::CorruptState => Err(RepositoryError::CorruptState),
            OutputMutation::NumericOverflow => Err(RepositoryError::NumericOverflow),
        }
    }

    /// Requests cancellation of a pending or executing run.
    ///
    /// A pending run is cancelled, abandoned, recorded, and released in one
    /// transaction. A running run advances to cancelling and asks the caller
    /// to signal its worker. Replaying a cancelling request does not advance
    /// revision or event sequence.
    ///
    /// # Errors
    ///
    /// Returns typed fence, state, overflow, corruption, actor, or SQLite
    /// failures.
    pub fn request_cancel(
        &self,
        run_id: RunKey,
        now_ms: u64,
    ) -> Result<(RunRecord, bool), RepositoryError> {
        let run_text = run_id.to_string();
        let now = sqlite_integer("now_ms", now_ms)?;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: Option<RawTransitionState> = transaction
                .query_row(
                    "SELECT r.state, r.cell_state, w.state,
                            r.project_id, r.lease_owner_id, r.fence, r.revision
                     FROM runs r
                     LEFT JOIN workspace_transactions w ON w.run_id = r.run_id
                     WHERE r.run_id = ?1",
                    [&run_text],
                    map_raw_transition_state,
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(CancelOutcome::NotFound);
            };
            let current = match current.parse() {
                Some(current) => current,
                None => return Ok(CancelOutcome::CorruptState),
            };
            if is_terminal(current.run_state) {
                return Ok(CancelOutcome::InvalidTransition);
            }
            if !transition_shape_is_valid(
                current.run_state,
                current.cell_state,
                current.transaction_state,
            ) {
                return Ok(CancelOutcome::CorruptState);
            }
            match check_fence(&transaction, &current.lease, now)? {
                FenceStatus::Current => {}
                FenceStatus::Rejected => return Ok(CancelOutcome::FenceRejected),
                FenceStatus::Corrupt => return Ok(CancelOutcome::CorruptState),
            }

            match current.run_state {
                RunState::Pending => {
                    let transition = TerminalTransition {
                        run_id: &run_text,
                        lease: &current.lease,
                        current_run_state: current.run_state,
                        current_cell_state: current.cell_state,
                        current_transaction_state: current.transaction_state,
                        current_revision: current.revision,
                        run_state: RunState::Cancelled,
                        cell_state: CellState::Cancelled,
                        transaction_state: TransactionState::Abandoned,
                        code: "CANCELLED",
                        event_kind: "cancelled",
                        environment: None,
                        kernel_generation: None,
                    };
                    match terminalize(&transaction, transition, now)? {
                        MutationOutcome::Applied => {}
                        MutationOutcome::FenceRejected => {
                            return Ok(CancelOutcome::FenceRejected);
                        }
                        MutationOutcome::InvalidTransition => {
                            return Ok(CancelOutcome::InvalidTransition);
                        }
                        MutationOutcome::CorruptState => {
                            return Ok(CancelOutcome::CorruptState);
                        }
                        MutationOutcome::NumericOverflow => {
                            return Ok(CancelOutcome::NumericOverflow);
                        }
                        MutationOutcome::NotFound | MutationOutcome::WorkspaceBindingMismatch => {
                            return Ok(CancelOutcome::CorruptState);
                        }
                    }
                    transaction.commit()?;
                    Ok(CancelOutcome::CancelledPending)
                }
                RunState::Running => {
                    let Some(next_revision) = checked_revision(current.revision) else {
                        return Ok(cancel_counter_failure(current.revision));
                    };
                    if transaction.execute(
                        "UPDATE runs
                         SET state = 'cancelling', revision = ?2, updated_at_ms = ?3
                         WHERE run_id = ?1 AND state = 'running' AND cell_state = 'running'
                           AND project_id = ?4 AND lease_owner_id = ?5 AND fence = ?6
                           AND revision = ?7",
                        params![
                            run_text,
                            next_revision,
                            now,
                            current.lease.project_id,
                            current.lease.owner_id,
                            current.lease.fence,
                            current.revision,
                        ],
                    )? != 1
                    {
                        return Ok(CancelOutcome::InvalidTransition);
                    }
                    match append_lifecycle_event(&transaction, &run_text, "cancel_requested", now)?
                    {
                        EventAppendOutcome::Applied(_) => {}
                        EventAppendOutcome::CorruptState => {
                            return Ok(CancelOutcome::CorruptState);
                        }
                        EventAppendOutcome::NumericOverflow => {
                            return Ok(CancelOutcome::NumericOverflow);
                        }
                    }
                    transaction.commit()?;
                    Ok(CancelOutcome::SignalWorker)
                }
                RunState::Cancelling => Ok(CancelOutcome::SignalWorker),
                RunState::Recovering
                | RunState::Succeeded
                | RunState::Failed
                | RunState::Cancelled
                | RunState::Interrupted => Ok(CancelOutcome::InvalidTransition),
            }
        })?;
        let should_signal = match outcome {
            CancelOutcome::SignalWorker => true,
            CancelOutcome::CancelledPending => false,
            CancelOutcome::NotFound => return Err(RepositoryError::NotFound),
            CancelOutcome::FenceRejected => return Err(RepositoryError::FenceRejected),
            CancelOutcome::InvalidTransition => return Err(RepositoryError::InvalidTransition),
            CancelOutcome::CorruptState => return Err(RepositoryError::CorruptState),
            CancelOutcome::NumericOverflow => return Err(RepositoryError::NumericOverflow),
        };
        Ok((self.run(run_id)?, should_signal))
    }

    /// Commits a result checkpoint and successful terminal state atomically.
    ///
    /// Checkpoint/CAS construction is complete before this method is called.
    /// The immediate writer transaction only validates durable ownership and
    /// binding, registers immutable metadata, advances state, appends the
    /// terminal event, and releases the lease. A cancelling run closes as
    /// cancelled without registering the supplied result.
    ///
    /// # Errors
    ///
    /// Returns typed fence, state, overflow, corruption, actor, or SQLite
    /// failures.
    pub fn finish_success(&self, input: FinishSuccess) -> Result<RunRecord, RepositoryError> {
        if input.kernel_generation == 0 {
            return Err(RepositoryError::InvalidTransition);
        }
        let run_id = input.run_id;
        let run_text = run_id.to_string();
        let checkpoint = PreparedCheckpoint::new(&input.result)?;
        let checkpoint_id = checkpoint.id.clone();
        let environment = input.environment.to_string();
        let generation = sqlite_integer("kernel_generation", input.kernel_generation)?;
        let now = sqlite_integer("now_ms", input.now_ms)?;
        let lease = PreparedLease::new(input.lease)?;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            match check_fence(&transaction, &lease, now)? {
                FenceStatus::Current => {}
                FenceStatus::Rejected => return Ok(MutationOutcome::FenceRejected),
                FenceStatus::Corrupt => return Ok(MutationOutcome::CorruptState),
            }
            let current: Option<RawSuccessState> = transaction
                .query_row(
                    "SELECT r.state, r.cell_state, w.state,
                            r.project_id, r.lease_owner_id, r.fence, r.revision,
                            r.transaction_id, w.transaction_id, w.project_id, w.fence
                     FROM runs r
                     LEFT JOIN workspace_transactions w ON w.run_id = r.run_id
                     WHERE r.run_id = ?1",
                    [&run_text],
                    map_raw_success_state,
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(MutationOutcome::NotFound);
            };
            let Some(current) = current.parse() else {
                return Ok(MutationOutcome::CorruptState);
            };
            if current.transition.lease != lease {
                return Ok(MutationOutcome::FenceRejected);
            }
            if is_terminal(current.transition.run_state) {
                return Ok(MutationOutcome::InvalidTransition);
            }
            if !transition_shape_is_valid(
                current.transition.run_state,
                current.transition.cell_state,
                current.transition.transaction_state,
            ) {
                return Ok(MutationOutcome::CorruptState);
            }
            if current.transition.run_state == RunState::Cancelling {
                let transition = TerminalTransition {
                    run_id: &run_text,
                    lease: &lease,
                    current_run_state: current.transition.run_state,
                    current_cell_state: current.transition.cell_state,
                    current_transaction_state: current.transition.transaction_state,
                    current_revision: current.transition.revision,
                    run_state: RunState::Cancelled,
                    cell_state: CellState::Cancelled,
                    transaction_state: TransactionState::Abandoned,
                    code: "CANCELLED",
                    event_kind: "cancelled",
                    environment: None,
                    kernel_generation: None,
                };
                let outcome = terminalize(&transaction, transition, now)?;
                if matches!(outcome, MutationOutcome::Applied) {
                    transaction.commit()?;
                }
                return Ok(outcome);
            }
            if current.transition.run_state != RunState::Running {
                return Ok(MutationOutcome::InvalidTransition);
            }
            let Some(next_revision) = checked_revision(current.transition.revision) else {
                return Ok(counter_failure(current.transition.revision));
            };

            if !register_checkpoint(&transaction, &checkpoint, now)? {
                return Ok(MutationOutcome::CorruptState);
            }
            if transaction.execute(
                "UPDATE runs
                 SET state = 'succeeded', cell_state = 'succeeded',
                     result_checkpoint_id = ?2, environment_digest = ?3,
                     kernel_generation = ?4, terminal_code = NULL,
                     revision = ?5, updated_at_ms = ?6
                 WHERE run_id = ?1 AND state = 'running' AND cell_state = 'running'
                   AND project_id = ?7 AND lease_owner_id = ?8 AND fence = ?9
                   AND revision = ?10",
                params![
                    run_text,
                    checkpoint_id,
                    environment,
                    generation,
                    next_revision,
                    now,
                    lease.project_id,
                    lease.owner_id,
                    lease.fence,
                    current.transition.revision,
                ],
            )? != 1
            {
                return Ok(MutationOutcome::InvalidTransition);
            }
            if transaction.execute(
                "UPDATE workspace_transactions
                 SET state = 'committed', result_checkpoint_id = ?2, updated_at_ms = ?3
                 WHERE transaction_id = ?1 AND run_id = ?4 AND project_id = ?5
                   AND fence = ?6 AND state = 'active'",
                params![
                    current.transaction_id,
                    checkpoint_id,
                    now,
                    run_text,
                    lease.project_id,
                    lease.fence,
                ],
            )? != 1
            {
                return Ok(MutationOutcome::InvalidTransition);
            }
            match append_lifecycle_event(&transaction, &run_text, "succeeded", now)? {
                EventAppendOutcome::Applied(_) => {}
                EventAppendOutcome::CorruptState => {
                    return Ok(MutationOutcome::CorruptState);
                }
                EventAppendOutcome::NumericOverflow => {
                    return Ok(MutationOutcome::NumericOverflow);
                }
            }
            if transaction.execute(
                "DELETE FROM project_leases
                 WHERE project_id = ?1 AND owner_id = ?2 AND fence = ?3",
                params![lease.project_id, lease.owner_id, lease.fence],
            )? != 1
            {
                return Ok(MutationOutcome::FenceRejected);
            }
            transaction.commit()?;
            Ok(MutationOutcome::Applied)
        })?;
        map_mutation(outcome)?;
        self.run(run_id)
    }

    /// Commits one closed non-success terminal result and releases its lease.
    ///
    /// Failed, cancelled, interrupted, and workspace-conflict outcomes update
    /// the run, cell attempt, workspace transaction, terminal event, and lease
    /// in one immediate transaction. A cancelling run always closes as
    /// cancelled regardless of the supplied disposition.
    ///
    /// # Errors
    ///
    /// Returns typed fence, state, overflow, corruption, actor, or SQLite
    /// failures.
    pub fn finish_terminal(&self, input: FinishTerminal) -> Result<RunRecord, RepositoryError> {
        let run_id = input.run_id;
        let run_text = run_id.to_string();
        let now = sqlite_integer("now_ms", input.now_ms)?;
        let generation = input
            .kernel_generation
            .map(|value| sqlite_integer("kernel_generation", value))
            .transpose()?;
        let environment = input.environment.map(|value| value.to_string());
        let code = input.code;
        let disposition = input.disposition;
        let lease = PreparedLease::new(input.lease)?;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            match check_fence(&transaction, &lease, now)? {
                FenceStatus::Current => {}
                FenceStatus::Rejected => return Ok(MutationOutcome::FenceRejected),
                FenceStatus::Corrupt => return Ok(MutationOutcome::CorruptState),
            }
            let current: Option<RawTransitionState> = transaction
                .query_row(
                    "SELECT r.state, r.cell_state, w.state,
                            r.project_id, r.lease_owner_id, r.fence, r.revision
                     FROM runs r
                     LEFT JOIN workspace_transactions w ON w.run_id = r.run_id
                     WHERE r.run_id = ?1",
                    [&run_text],
                    map_raw_transition_state,
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(MutationOutcome::NotFound);
            };
            let current = match current.parse() {
                Some(current) => current,
                None => return Ok(MutationOutcome::CorruptState),
            };
            if current.lease != lease {
                return Ok(MutationOutcome::FenceRejected);
            }
            if is_terminal(current.run_state) {
                return Ok(MutationOutcome::InvalidTransition);
            }
            if !transition_shape_is_valid(
                current.run_state,
                current.cell_state,
                current.transaction_state,
            ) {
                return Ok(MutationOutcome::CorruptState);
            }
            let effective = if current.run_state == RunState::Cancelling {
                FinishDisposition::Cancelled
            } else {
                disposition
            };
            let (run_state, cell_state, transaction_state, event_kind) = match effective {
                FinishDisposition::Failed => (
                    RunState::Failed,
                    CellState::Failed,
                    TransactionState::Abandoned,
                    "failed",
                ),
                FinishDisposition::Cancelled => (
                    RunState::Cancelled,
                    CellState::Cancelled,
                    TransactionState::Abandoned,
                    "cancelled",
                ),
                FinishDisposition::Interrupted => (
                    RunState::Interrupted,
                    CellState::Interrupted,
                    TransactionState::Abandoned,
                    "interrupted",
                ),
                FinishDisposition::Conflict => (
                    RunState::Failed,
                    CellState::Failed,
                    TransactionState::Conflict,
                    "workspace_conflict",
                ),
            };
            let transition = TerminalTransition {
                run_id: &run_text,
                lease: &lease,
                current_run_state: current.run_state,
                current_cell_state: current.cell_state,
                current_transaction_state: current.transaction_state,
                current_revision: current.revision,
                run_state,
                cell_state,
                transaction_state,
                code: &code,
                event_kind,
                environment: environment.as_deref(),
                kernel_generation: generation,
            };
            let outcome = terminalize(&transaction, transition, now)?;
            if matches!(outcome, MutationOutcome::Applied) {
                transaction.commit()?;
            }
            Ok(outcome)
        })?;
        map_mutation(outcome)?;
        self.run(run_id)
    }

    /// Atomically interrupts every durably incomplete run after a restart.
    ///
    /// Candidates are selected by `(created_at_ms, run_id)` with an explicit
    /// [`MAX_RECONCILE_RUNS`] all-or-nothing bound. Pending, running, and
    /// cancelling runs first record `recovering`; already-recovering runs keep
    /// their existing marker. Every candidate then becomes interrupted, its
    /// workspace transaction is abandoned, and only its exact persisted
    /// owner/fence lease is released. A valid nonmatching current lease is
    /// preserved.
    ///
    /// # Errors
    ///
    /// Returns typed bound, overflow, corruption, actor, or SQLite failures.
    /// Any failure rolls back the complete reconciliation transaction.
    pub fn reconcile_incomplete(&self, now_ms: u64) -> Result<u64, RepositoryError> {
        let now = sqlite_integer("now_ms", now_ms)?;
        let selection_limit = i64::from(MAX_RECONCILE_RUNS) + 1;
        let maximum =
            usize::try_from(MAX_RECONCILE_RUNS).map_err(|_| RepositoryError::NumericOverflow)?;
        let outcome = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let raw = {
                let mut statement = transaction.prepare(
                    "SELECT r.run_id, r.project_id, r.transaction_id, r.cell_id,
                            r.cell_revision, r.lease_owner_id, r.fence,
                            r.lease_expires_at_ms, r.state, r.cell_state,
                            r.last_sequence, r.revision, r.created_at_ms, r.updated_at_ms,
                            w.transaction_id, w.project_id, w.fence, w.state
                     FROM runs r
                     LEFT JOIN workspace_transactions w ON w.run_id = r.run_id
                     WHERE r.state IN ('pending', 'running', 'cancelling', 'recovering')
                     ORDER BY r.created_at_ms ASC, r.run_id ASC
                     LIMIT ?1",
                )?;
                let rows = statement.query_map([selection_limit], map_raw_reconcile_run)?;
                let mut raw = Vec::new();
                for row in rows {
                    raw.push(row?);
                }
                raw
            };
            if raw.len() > maximum {
                return Ok(ReconcileOutcome::LimitExceeded);
            }

            let count = match u64::try_from(raw.len()) {
                Ok(count) => count,
                Err(_) => return Ok(ReconcileOutcome::NumericOverflow),
            };
            for raw_run in raw {
                let Some(run) = raw_run.parse() else {
                    return Ok(ReconcileOutcome::CorruptState);
                };
                let lease: Option<(String, i64, i64)> = transaction
                    .query_row(
                        "SELECT owner_id, fence, expires_at_ms
                         FROM project_leases WHERE project_id = ?1",
                        [&run.project_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                if lease.is_some_and(|(owner_id, fence, expires_at_ms)| {
                    decode_lease_owner_key("owner_id", &owner_id).is_err()
                        || decode_positive("fence", fence).is_err()
                        || decode_non_negative("expires_at_ms", expires_at_ms).is_err()
                }) {
                    return Ok(ReconcileOutcome::CorruptState);
                }

                let mut revision = run.revision;
                if run.run_state != RunState::Recovering {
                    let Some(recovering_revision) = checked_revision(revision) else {
                        return Ok(reconcile_counter_failure(revision));
                    };
                    if transaction.execute(
                        "UPDATE runs
                         SET state = 'recovering', cell_state = 'recovering',
                             revision = ?2, updated_at_ms = ?3
                         WHERE run_id = ?1 AND state = ?4 AND cell_state = ?5
                           AND project_id = ?6 AND lease_owner_id = ?7 AND fence = ?8
                           AND revision = ?9",
                        params![
                            run.run_id,
                            recovering_revision,
                            now,
                            run.run_state.as_str(),
                            run.cell_state.as_str(),
                            run.project_id,
                            run.owner_id,
                            run.fence,
                            revision,
                        ],
                    )? != 1
                    {
                        return Ok(ReconcileOutcome::CorruptState);
                    }
                    match append_lifecycle_event(&transaction, &run.run_id, "recovering", now)? {
                        EventAppendOutcome::Applied(_) => {}
                        EventAppendOutcome::CorruptState => {
                            return Ok(ReconcileOutcome::CorruptState);
                        }
                        EventAppendOutcome::NumericOverflow => {
                            return Ok(ReconcileOutcome::NumericOverflow);
                        }
                    }
                    let Some(after_event_revision) = recovering_revision.checked_add(1) else {
                        return Ok(ReconcileOutcome::NumericOverflow);
                    };
                    revision = after_event_revision;
                }

                let Some(interrupted_revision) = checked_revision(revision) else {
                    return Ok(reconcile_counter_failure(revision));
                };
                if transaction.execute(
                    "UPDATE runs
                     SET state = 'interrupted', cell_state = 'interrupted',
                         terminal_code = 'PROCESS_DIED', revision = ?2,
                         updated_at_ms = ?3
                     WHERE run_id = ?1 AND state = 'recovering'
                       AND cell_state = 'recovering' AND project_id = ?4
                       AND lease_owner_id = ?5 AND fence = ?6 AND revision = ?7",
                    params![
                        run.run_id,
                        interrupted_revision,
                        now,
                        run.project_id,
                        run.owner_id,
                        run.fence,
                        revision,
                    ],
                )? != 1
                {
                    return Ok(ReconcileOutcome::CorruptState);
                }
                if transaction.execute(
                    "UPDATE workspace_transactions
                     SET state = 'abandoned', updated_at_ms = ?2
                     WHERE transaction_id = ?1 AND run_id = ?3 AND project_id = ?4
                       AND fence = ?5 AND state = ?6",
                    params![
                        run.transaction_id,
                        now,
                        run.run_id,
                        run.project_id,
                        run.fence,
                        run.transaction_state.as_str(),
                    ],
                )? != 1
                {
                    return Ok(ReconcileOutcome::CorruptState);
                }
                match append_lifecycle_event(&transaction, &run.run_id, "interrupted", now)? {
                    EventAppendOutcome::Applied(_) => {}
                    EventAppendOutcome::CorruptState => {
                        return Ok(ReconcileOutcome::CorruptState);
                    }
                    EventAppendOutcome::NumericOverflow => {
                        return Ok(ReconcileOutcome::NumericOverflow);
                    }
                }
                transaction.execute(
                    "DELETE FROM project_leases
                     WHERE project_id = ?1 AND owner_id = ?2 AND fence = ?3",
                    params![run.project_id, run.owner_id, run.fence],
                )?;
            }
            transaction.commit()?;
            Ok(ReconcileOutcome::Applied(count))
        })?;
        match outcome {
            ReconcileOutcome::Applied(count) => Ok(count),
            ReconcileOutcome::CorruptState => Err(RepositoryError::CorruptState),
            ReconcileOutcome::NumericOverflow => Err(RepositoryError::NumericOverflow),
            ReconcileOutcome::LimitExceeded => Err(RepositoryError::ReconciliationLimitExceeded {
                limit: MAX_RECONCILE_RUNS,
            }),
        }
    }

    /// Returns one durable run by its storage identity.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError::NotFound`] or typed corruption, actor, and
    /// SQLite failures.
    pub fn run(&self, run_id: RunKey) -> Result<RunRecord, RepositoryError> {
        let run_text = run_id.to_string();
        let raw = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            connection
                .query_row(RUN_SELECT, [&run_text], map_raw_run)
                .optional()
                .map_err(StoreError::from)
        })?;
        raw.ok_or(RepositoryError::NotFound)?.parse()
    }

    /// Returns one durable checkpoint with entries ordered by path.
    ///
    /// Metadata and every entry are decoded through the closed storage codecs.
    /// The stored entry count must exactly match the selected rows.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryError::NotFound`] or typed corruption, actor, and
    /// SQLite failures.
    pub fn checkpoint(
        &self,
        checkpoint_id: CheckpointKey,
    ) -> Result<CheckpointRecord, RepositoryError> {
        let checkpoint_text = checkpoint_id.to_string();
        let raw = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let metadata: Option<RawCheckpoint> = connection
                .query_row(
                    "SELECT manifest_digest, manifest_length, backend, fidelity,
                            git_context_digest, entry_count, total_file_bytes, created_at_ms
                     FROM checkpoints WHERE checkpoint_id = ?1",
                    [&checkpoint_text],
                    map_raw_checkpoint,
                )
                .optional()?;
            let Some(metadata) = metadata else {
                return Ok(None);
            };
            let mut statement = connection.prepare(
                "SELECT path, kind, object_digest, object_length, is_executable
                 FROM checkpoint_entries
                 WHERE checkpoint_id = ?1
                 ORDER BY path ASC",
            )?;
            let rows = statement.query_map([&checkpoint_text], map_raw_checkpoint_entry)?;
            let mut entries = Vec::new();
            for row in rows {
                entries.push(row?);
            }
            Ok(Some((metadata, entries)))
        })?;
        let (metadata, entries) = raw.ok_or(RepositoryError::NotFound)?;
        metadata.parse(checkpoint_id, entries)
    }

    /// Returns a sequence-ordered, SQL-bounded event page after a cursor.
    ///
    /// `limit` must be in `1..=MAX_WATCH_EVENTS`. The returned vector never
    /// materializes records beyond that bound.
    ///
    /// # Errors
    ///
    /// Returns typed limit, overflow, corruption, actor, or SQLite failures.
    pub fn watch(
        &self,
        run_id: RunKey,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Vec<StoredEvent>, RepositoryError> {
        if limit == 0 || limit > MAX_WATCH_EVENTS {
            return Err(RepositoryError::InvalidTransition);
        }
        let run_text = run_id.to_string();
        let after = sqlite_integer("after_sequence", after_sequence)?;
        let sql_limit = i64::from(limit);
        let capacity = usize::try_from(limit).map_err(|_| RepositoryError::NumericOverflow)?;
        let raw = self.handle.call(STORE_REPLY_TIMEOUT, move |connection| {
            let mut statement = connection.prepare(
                "SELECT e.sequence, e.kind, e.worker_sequence, e.stream,
                        e.blob_digest, e.blob_length, e.occurred_at_ms,
                        o.stream, o.blob_digest, o.blob_length
                 FROM events e
                 LEFT JOIN output_chunks o
                   ON o.run_id = e.run_id AND o.event_sequence = e.sequence
                 WHERE e.run_id = ?1 AND e.sequence > ?2
                 ORDER BY e.sequence ASC
                 LIMIT ?3",
            )?;
            let rows = statement.query_map(params![run_text, after, sql_limit], map_raw_event)?;
            let mut events = Vec::with_capacity(capacity);
            for row in rows {
                events.push(row?);
            }
            Ok(events)
        })?;
        raw.into_iter().map(RawEvent::parse).collect()
    }
}

enum BeginDbOutcome {
    Created,
    Replay(String),
    IdempotencyConflict,
    LeaseConflict,
    CorruptState,
    NumericOverflow,
}

#[derive(Clone, Copy)]
enum MutationOutcome {
    Applied,
    FenceRejected,
    NotFound,
    InvalidTransition,
    WorkspaceBindingMismatch,
    CorruptState,
    NumericOverflow,
}

#[derive(Clone, Copy)]
enum EventAppendOutcome {
    Applied(i64),
    CorruptState,
    NumericOverflow,
}

#[derive(Clone, Copy)]
enum OutputMutation {
    Applied(i64),
    FenceRejected,
    NotFound,
    InvalidTransition,
    BudgetExceeded,
    CorruptState,
    NumericOverflow,
}

#[derive(Clone, Copy)]
enum CancelOutcome {
    SignalWorker,
    CancelledPending,
    NotFound,
    FenceRejected,
    InvalidTransition,
    CorruptState,
    NumericOverflow,
}

enum ReconcileOutcome {
    Applied(u64),
    CorruptState,
    NumericOverflow,
    LimitExceeded,
}

fn map_mutation(outcome: MutationOutcome) -> Result<(), RepositoryError> {
    match outcome {
        MutationOutcome::Applied => Ok(()),
        MutationOutcome::FenceRejected => Err(RepositoryError::FenceRejected),
        MutationOutcome::NotFound => Err(RepositoryError::NotFound),
        MutationOutcome::InvalidTransition => Err(RepositoryError::InvalidTransition),
        MutationOutcome::WorkspaceBindingMismatch => Err(RepositoryError::WorkspaceBindingMismatch),
        MutationOutcome::CorruptState => Err(RepositoryError::CorruptState),
        MutationOutcome::NumericOverflow => Err(RepositoryError::NumericOverflow),
    }
}

fn sqlite_integer(column: &'static str, value: u64) -> Result<i64, RepositoryError> {
    encode_integer(column, value).map_err(|_| RepositoryError::NumericOverflow)
}

fn decode_storage<T>(result: Result<T, CorruptStorageError>) -> Result<T, RepositoryError> {
    result.map_err(|_| RepositoryError::CorruptState)
}

fn checked_revision(revision: i64) -> Option<i64> {
    if revision <= 0 {
        None
    } else {
        revision.checked_add(1)
    }
}

fn counter_failure(value: i64) -> MutationOutcome {
    if value <= 0 {
        MutationOutcome::CorruptState
    } else {
        MutationOutcome::NumericOverflow
    }
}

fn cancel_counter_failure(value: i64) -> CancelOutcome {
    if value <= 0 {
        CancelOutcome::CorruptState
    } else {
        CancelOutcome::NumericOverflow
    }
}

fn reconcile_counter_failure(value: i64) -> ReconcileOutcome {
    if value <= 0 {
        ReconcileOutcome::CorruptState
    } else {
        ReconcileOutcome::NumericOverflow
    }
}

#[derive(Eq, PartialEq)]
struct PreparedLease {
    project_id: String,
    owner_id: String,
    fence: i64,
}

impl PreparedLease {
    fn new(lease: LeaseGrant) -> Result<Self, RepositoryError> {
        Ok(Self {
            project_id: lease.project_id.to_string(),
            owner_id: lease.owner_id.to_string(),
            fence: sqlite_integer("fence", lease.fence.value())?,
        })
    }

    fn matches_stored(
        &self,
        project_id: &str,
        owner_id: &str,
        fence: i64,
    ) -> Result<bool, CorruptStorageError> {
        decode_project_key("project_id", project_id)?;
        decode_lease_owner_key("lease_owner_id", owner_id)?;
        decode_positive("fence", fence)?;
        Ok(self.project_id == project_id && self.owner_id == owner_id && self.fence == fence)
    }
}

struct RawTransitionState {
    run_state: String,
    cell_state: String,
    transaction_state: Option<String>,
    project_id: String,
    owner_id: String,
    fence: i64,
    revision: i64,
}

fn map_raw_transition_state(row: &Row<'_>) -> rusqlite::Result<RawTransitionState> {
    Ok(RawTransitionState {
        run_state: row.get(0)?,
        cell_state: row.get(1)?,
        transaction_state: row.get(2)?,
        project_id: row.get(3)?,
        owner_id: row.get(4)?,
        fence: row.get(5)?,
        revision: row.get(6)?,
    })
}

struct TransitionState {
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
    lease: PreparedLease,
    revision: i64,
}

impl RawTransitionState {
    fn parse(self) -> Option<TransitionState> {
        let transaction_state = self.transaction_state?;
        decode_project_key("project_id", &self.project_id).ok()?;
        decode_lease_owner_key("lease_owner_id", &self.owner_id).ok()?;
        decode_positive("fence", self.fence).ok()?;
        decode_positive("revision", self.revision).ok()?;
        Some(TransitionState {
            run_state: RunState::decode("state", &self.run_state).ok()?,
            cell_state: CellState::decode("cell_state", &self.cell_state).ok()?,
            transaction_state: TransactionState::decode("transaction_state", &transaction_state)
                .ok()?,
            lease: PreparedLease {
                project_id: self.project_id,
                owner_id: self.owner_id,
                fence: self.fence,
            },
            revision: self.revision,
        })
    }
}

struct RawSuccessState {
    transition: RawTransitionState,
    run_transaction_id: String,
    workspace_transaction_id: Option<String>,
    workspace_project_id: Option<String>,
    workspace_fence: Option<i64>,
}

fn map_raw_success_state(row: &Row<'_>) -> rusqlite::Result<RawSuccessState> {
    Ok(RawSuccessState {
        transition: RawTransitionState {
            run_state: row.get(0)?,
            cell_state: row.get(1)?,
            transaction_state: row.get(2)?,
            project_id: row.get(3)?,
            owner_id: row.get(4)?,
            fence: row.get(5)?,
            revision: row.get(6)?,
        },
        run_transaction_id: row.get(7)?,
        workspace_transaction_id: row.get(8)?,
        workspace_project_id: row.get(9)?,
        workspace_fence: row.get(10)?,
    })
}

struct SuccessState {
    transition: TransitionState,
    transaction_id: String,
}

impl RawSuccessState {
    fn parse(self) -> Option<SuccessState> {
        let transition = self.transition.parse()?;
        let workspace_transaction_id = self.workspace_transaction_id?;
        let workspace_project_id = self.workspace_project_id?;
        let workspace_fence = self.workspace_fence?;
        let run_transaction =
            decode_transaction_key("transaction_id", &self.run_transaction_id).ok()?;
        let workspace_transaction =
            decode_transaction_key("transaction_id", &workspace_transaction_id).ok()?;
        let run_project = decode_project_key("project_id", &transition.lease.project_id).ok()?;
        let workspace_project = decode_project_key("project_id", &workspace_project_id).ok()?;
        let run_fence = decode_positive("fence", transition.lease.fence).ok()?;
        let workspace_fence = decode_positive("fence", workspace_fence).ok()?;
        if run_transaction != workspace_transaction
            || workspace_project != run_project
            || workspace_fence != run_fence
        {
            return None;
        }
        Some(SuccessState {
            transition,
            transaction_id: self.run_transaction_id,
        })
    }
}

fn is_terminal(state: RunState) -> bool {
    matches!(
        state,
        RunState::Succeeded | RunState::Failed | RunState::Cancelled | RunState::Interrupted
    )
}

fn transition_shape_is_valid(
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
) -> bool {
    match run_state {
        RunState::Pending => {
            cell_state == CellState::Queued
                && matches!(
                    transaction_state,
                    TransactionState::Prepared | TransactionState::Active
                )
        }
        RunState::Running | RunState::Cancelling => {
            cell_state == CellState::Running && transaction_state == TransactionState::Active
        }
        RunState::Recovering => {
            cell_state == CellState::Recovering && transaction_state == TransactionState::Active
        }
        RunState::Succeeded => {
            cell_state == CellState::Succeeded && transaction_state == TransactionState::Committed
        }
        RunState::Failed => {
            cell_state == CellState::Failed
                && matches!(
                    transaction_state,
                    TransactionState::Abandoned | TransactionState::Conflict
                )
        }
        RunState::Cancelled => {
            cell_state == CellState::Cancelled && transaction_state == TransactionState::Abandoned
        }
        RunState::Interrupted => {
            cell_state == CellState::Interrupted && transaction_state == TransactionState::Abandoned
        }
    }
}

fn run_shape_is_valid(
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
) -> bool {
    if run_state == RunState::Recovering {
        cell_state == CellState::Recovering
            && matches!(
                transaction_state,
                TransactionState::Prepared | TransactionState::Active
            )
    } else {
        transition_shape_is_valid(run_state, cell_state, transaction_state)
    }
}

struct RawReconcileRun {
    run_id: String,
    project_id: String,
    transaction_id: String,
    cell_id: String,
    cell_revision: i64,
    owner_id: String,
    fence: i64,
    lease_expires_at_ms: i64,
    run_state: String,
    cell_state: String,
    last_sequence: i64,
    revision: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    workspace_transaction_id: Option<String>,
    workspace_project_id: Option<String>,
    workspace_fence: Option<i64>,
    transaction_state: Option<String>,
}

fn map_raw_reconcile_run(row: &Row<'_>) -> rusqlite::Result<RawReconcileRun> {
    Ok(RawReconcileRun {
        run_id: row.get(0)?,
        project_id: row.get(1)?,
        transaction_id: row.get(2)?,
        cell_id: row.get(3)?,
        cell_revision: row.get(4)?,
        owner_id: row.get(5)?,
        fence: row.get(6)?,
        lease_expires_at_ms: row.get(7)?,
        run_state: row.get(8)?,
        cell_state: row.get(9)?,
        last_sequence: row.get(10)?,
        revision: row.get(11)?,
        created_at_ms: row.get(12)?,
        updated_at_ms: row.get(13)?,
        workspace_transaction_id: row.get(14)?,
        workspace_project_id: row.get(15)?,
        workspace_fence: row.get(16)?,
        transaction_state: row.get(17)?,
    })
}

struct ReconcileRun {
    run_id: String,
    project_id: String,
    transaction_id: String,
    owner_id: String,
    fence: i64,
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
    revision: i64,
}

impl RawReconcileRun {
    fn parse(self) -> Option<ReconcileRun> {
        let workspace_transaction_id = self.workspace_transaction_id?;
        let workspace_project_id = self.workspace_project_id?;
        let workspace_fence = self.workspace_fence?;
        let transaction_state = self.transaction_state?;
        let run_id = decode_run_key("run_id", &self.run_id).ok()?;
        let project_id = decode_project_key("project_id", &self.project_id).ok()?;
        let transaction_id = decode_transaction_key("transaction_id", &self.transaction_id).ok()?;
        decode_cell_key("cell_id", &self.cell_id).ok()?;
        decode_positive("cell_revision", self.cell_revision).ok()?;
        decode_lease_owner_key("lease_owner_id", &self.owner_id).ok()?;
        let fence = decode_positive("fence", self.fence).ok()?;
        decode_non_negative("lease_expires_at_ms", self.lease_expires_at_ms).ok()?;
        let run_state = RunState::decode("state", &self.run_state).ok()?;
        let cell_state = CellState::decode("cell_state", &self.cell_state).ok()?;
        decode_positive("last_sequence", self.last_sequence).ok()?;
        decode_positive("revision", self.revision).ok()?;
        decode_non_negative("created_at_ms", self.created_at_ms).ok()?;
        decode_non_negative("updated_at_ms", self.updated_at_ms).ok()?;
        let stored_transaction =
            decode_transaction_key("transaction_id", &workspace_transaction_id).ok()?;
        let stored_project = decode_project_key("project_id", &workspace_project_id).ok()?;
        let stored_fence = decode_positive("fence", workspace_fence).ok()?;
        let transaction_state =
            TransactionState::decode("transaction_state", &transaction_state).ok()?;
        if transaction_id != stored_transaction
            || project_id != stored_project
            || fence != stored_fence
            || !reconcile_shape_is_valid(run_state, cell_state, transaction_state)
        {
            return None;
        }
        Some(ReconcileRun {
            run_id: run_id.to_string(),
            project_id: project_id.to_string(),
            transaction_id: transaction_id.to_string(),
            owner_id: self.owner_id,
            fence: self.fence,
            run_state,
            cell_state,
            transaction_state,
            revision: self.revision,
        })
    }
}

fn reconcile_shape_is_valid(
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
) -> bool {
    match run_state {
        RunState::Pending => {
            cell_state == CellState::Queued
                && matches!(
                    transaction_state,
                    TransactionState::Prepared | TransactionState::Active
                )
        }
        RunState::Running | RunState::Cancelling => {
            cell_state == CellState::Running && transaction_state == TransactionState::Active
        }
        RunState::Recovering => {
            cell_state == CellState::Recovering
                && matches!(
                    transaction_state,
                    TransactionState::Prepared | TransactionState::Active
                )
        }
        RunState::Succeeded | RunState::Failed | RunState::Cancelled | RunState::Interrupted => {
            false
        }
    }
}

struct PreparedCheckpointEntry {
    path: String,
    kind: &'static str,
    object_digest: String,
    object_length: i64,
    is_executable: i64,
}

struct PreparedCheckpoint {
    id: String,
    manifest_digest: String,
    manifest_length: i64,
    backend: &'static str,
    fidelity: &'static str,
    git_context_digest: Option<String>,
    entry_count: i64,
    total_file_bytes: i64,
    entries: Vec<PreparedCheckpointEntry>,
}

impl PreparedCheckpoint {
    fn new(checkpoint: &CheckpointRecord) -> Result<Self, RepositoryError> {
        let entry_count = u64::try_from(checkpoint.entries.len())
            .map_err(|_| RepositoryError::NumericOverflow)?;
        let mut entries = checkpoint
            .entries
            .iter()
            .map(|entry| {
                Ok(PreparedCheckpointEntry {
                    path: entry.path.clone(),
                    kind: entry.kind.as_str(),
                    object_digest: entry.object.digest.to_string(),
                    object_length: sqlite_integer("object_length", entry.object.length)?,
                    is_executable: i64::from(entry.is_executable),
                })
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            id: checkpoint.id.to_string(),
            manifest_digest: checkpoint.manifest.digest.to_string(),
            manifest_length: sqlite_integer("manifest_length", checkpoint.manifest.length)?,
            backend: checkpoint.backend.as_str(),
            fidelity: checkpoint.fidelity.as_str(),
            git_context_digest: checkpoint.git_context.map(|value| value.to_string()),
            entry_count: sqlite_integer("entry_count", entry_count)?,
            total_file_bytes: sqlite_integer("total_file_bytes", checkpoint.total_file_bytes)?,
            entries,
        })
    }
}

#[derive(Clone, Copy)]
enum FenceStatus {
    Current,
    Rejected,
    Corrupt,
}

fn check_fence(
    transaction: &Transaction<'_>,
    lease: &PreparedLease,
    now_ms: i64,
) -> Result<FenceStatus, StoreError> {
    let current: Option<(String, i64, i64)> = transaction
        .query_row(
            "SELECT owner_id, fence, expires_at_ms
             FROM project_leases WHERE project_id = ?1",
            [&lease.project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((owner_id, fence, expires_at_ms)) = current else {
        return Ok(FenceStatus::Rejected);
    };
    if decode_lease_owner_key("owner_id", &owner_id).is_err()
        || decode_positive("fence", fence).is_err()
        || decode_non_negative("expires_at_ms", expires_at_ms).is_err()
    {
        return Ok(FenceStatus::Corrupt);
    }
    if owner_id == lease.owner_id && fence == lease.fence && expires_at_ms > now_ms {
        Ok(FenceStatus::Current)
    } else {
        Ok(FenceStatus::Rejected)
    }
}

struct TerminalTransition<'a> {
    run_id: &'a str,
    lease: &'a PreparedLease,
    current_run_state: RunState,
    current_cell_state: CellState,
    current_transaction_state: TransactionState,
    current_revision: i64,
    run_state: RunState,
    cell_state: CellState,
    transaction_state: TransactionState,
    code: &'a str,
    event_kind: &'a str,
    environment: Option<&'a str>,
    kernel_generation: Option<i64>,
}

fn terminalize(
    transaction: &Transaction<'_>,
    transition: TerminalTransition<'_>,
    now_ms: i64,
) -> Result<MutationOutcome, StoreError> {
    let Some(next_revision) = checked_revision(transition.current_revision) else {
        return Ok(counter_failure(transition.current_revision));
    };
    if transaction.execute(
        "UPDATE runs
         SET state = ?2, cell_state = ?3, terminal_code = ?4,
             environment_digest = COALESCE(?5, environment_digest),
             kernel_generation = COALESCE(?6, kernel_generation),
             revision = ?7, updated_at_ms = ?8
         WHERE run_id = ?1 AND state = ?9 AND cell_state = ?10
           AND project_id = ?11 AND lease_owner_id = ?12 AND fence = ?13
           AND revision = ?14",
        params![
            transition.run_id,
            transition.run_state.as_str(),
            transition.cell_state.as_str(),
            transition.code,
            transition.environment,
            transition.kernel_generation,
            next_revision,
            now_ms,
            transition.current_run_state.as_str(),
            transition.current_cell_state.as_str(),
            transition.lease.project_id,
            transition.lease.owner_id,
            transition.lease.fence,
            transition.current_revision,
        ],
    )? != 1
    {
        return Ok(MutationOutcome::InvalidTransition);
    }
    if transaction.execute(
        "UPDATE workspace_transactions
         SET state = ?2, updated_at_ms = ?3
         WHERE run_id = ?1 AND project_id = ?4 AND fence = ?5 AND state = ?6",
        params![
            transition.run_id,
            transition.transaction_state.as_str(),
            now_ms,
            transition.lease.project_id,
            transition.lease.fence,
            transition.current_transaction_state.as_str(),
        ],
    )? != 1
    {
        return Ok(MutationOutcome::InvalidTransition);
    }
    match append_lifecycle_event(
        transaction,
        transition.run_id,
        transition.event_kind,
        now_ms,
    )? {
        EventAppendOutcome::Applied(_) => {}
        EventAppendOutcome::CorruptState => return Ok(MutationOutcome::CorruptState),
        EventAppendOutcome::NumericOverflow => return Ok(MutationOutcome::NumericOverflow),
    }
    if transaction.execute(
        "DELETE FROM project_leases
         WHERE project_id = ?1 AND owner_id = ?2 AND fence = ?3",
        params![
            transition.lease.project_id,
            transition.lease.owner_id,
            transition.lease.fence,
        ],
    )? != 1
    {
        return Ok(MutationOutcome::FenceRejected);
    }
    Ok(MutationOutcome::Applied)
}

fn register_checkpoint(
    transaction: &Transaction<'_>,
    checkpoint: &PreparedCheckpoint,
    now_ms: i64,
) -> Result<bool, StoreError> {
    let inserted = transaction.execute(
        "INSERT INTO checkpoints (
            checkpoint_id, manifest_digest, manifest_length, backend, fidelity,
            git_context_digest, entry_count, total_file_bytes, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(checkpoint_id) DO NOTHING",
        params![
            checkpoint.id,
            checkpoint.manifest_digest,
            checkpoint.manifest_length,
            checkpoint.backend,
            checkpoint.fidelity,
            checkpoint.git_context_digest,
            checkpoint.entry_count,
            checkpoint.total_file_bytes,
            now_ms,
        ],
    )?;
    if inserted == 0 {
        return checkpoint_matches(transaction, checkpoint);
    }
    for entry in &checkpoint.entries {
        if transaction.execute(
            "INSERT INTO checkpoint_entries (
                checkpoint_id, path, kind, object_digest, object_length, is_executable
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(checkpoint_id, path) DO NOTHING",
            params![
                checkpoint.id,
                entry.path,
                entry.kind,
                entry.object_digest,
                entry.object_length,
                entry.is_executable,
            ],
        )? != 1
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn checkpoint_matches(
    transaction: &Transaction<'_>,
    checkpoint: &PreparedCheckpoint,
) -> Result<bool, StoreError> {
    let metadata: (String, i64, String, String, Option<String>, i64, i64) = transaction.query_row(
        "SELECT manifest_digest, manifest_length, backend, fidelity,
                    git_context_digest, entry_count, total_file_bytes
             FROM checkpoints WHERE checkpoint_id = ?1",
        [&checkpoint.id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    if metadata
        != (
            checkpoint.manifest_digest.clone(),
            checkpoint.manifest_length,
            checkpoint.backend.to_owned(),
            checkpoint.fidelity.to_owned(),
            checkpoint.git_context_digest.clone(),
            checkpoint.entry_count,
            checkpoint.total_file_bytes,
        )
    {
        return Ok(false);
    }

    let stored_entries = {
        let mut statement = transaction.prepare(
            "SELECT path, kind, object_digest, object_length, is_executable
             FROM checkpoint_entries
             WHERE checkpoint_id = ?1
             ORDER BY path ASC",
        )?;
        let rows = statement.query_map([&checkpoint.id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut stored = Vec::new();
        for row in rows {
            stored.push(row?);
        }
        stored
    };
    if stored_entries.len() != checkpoint.entries.len() {
        return Ok(false);
    }
    Ok(stored_entries
        .iter()
        .zip(&checkpoint.entries)
        .all(|(stored, expected)| {
            stored.0 == expected.path
                && stored.1 == expected.kind
                && stored.2 == expected.object_digest
                && stored.3 == expected.object_length
                && stored.4 == expected.is_executable
        }))
}

fn append_lifecycle_event(
    transaction: &Transaction<'_>,
    run_id: &str,
    kind: &str,
    now_ms: i64,
) -> Result<EventAppendOutcome, StoreError> {
    append_event(transaction, run_id, EventData::lifecycle(kind), now_ms)
}

struct EventData<'a> {
    kind: &'a str,
    worker_sequence: Option<i64>,
    stream: Option<&'a str>,
    blob_digest: Option<&'a str>,
    blob_length: Option<i64>,
}

impl<'a> EventData<'a> {
    fn lifecycle(kind: &'a str) -> Self {
        Self {
            kind,
            worker_sequence: None,
            stream: None,
            blob_digest: None,
            blob_length: None,
        }
    }

    fn output(
        worker_sequence: i64,
        stream: &'a str,
        blob_digest: &'a str,
        blob_length: i64,
    ) -> Self {
        Self {
            kind: "output",
            worker_sequence: Some(worker_sequence),
            stream: Some(stream),
            blob_digest: Some(blob_digest),
            blob_length: Some(blob_length),
        }
    }
}

fn append_event(
    transaction: &Transaction<'_>,
    run_id: &str,
    event: EventData<'_>,
    now_ms: i64,
) -> Result<EventAppendOutcome, StoreError> {
    let current: Option<(i64, i64)> = transaction
        .query_row(
            "SELECT last_sequence, revision FROM runs WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((last_sequence, revision)) = current else {
        return Ok(EventAppendOutcome::CorruptState);
    };
    if last_sequence <= 0 || revision <= 0 {
        return Ok(EventAppendOutcome::CorruptState);
    }
    let Some(sequence) = last_sequence.checked_add(1) else {
        return Ok(EventAppendOutcome::NumericOverflow);
    };
    let Some(next_revision) = revision.checked_add(1) else {
        return Ok(EventAppendOutcome::NumericOverflow);
    };
    if transaction.execute(
        "UPDATE runs
         SET last_sequence = ?2, revision = ?3, updated_at_ms = ?4
         WHERE run_id = ?1 AND last_sequence = ?5 AND revision = ?6",
        params![
            run_id,
            sequence,
            next_revision,
            now_ms,
            last_sequence,
            revision,
        ],
    )? != 1
    {
        return Ok(EventAppendOutcome::CorruptState);
    }
    transaction.execute(
        "INSERT INTO events (
             run_id, sequence, kind, worker_sequence, stream,
             blob_digest, blob_length, occurred_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run_id,
            sequence,
            event.kind,
            event.worker_sequence,
            event.stream,
            event.blob_digest,
            event.blob_length,
            now_ms,
        ],
    )?;
    Ok(EventAppendOutcome::Applied(sequence))
}

const RUN_SELECT: &str = "
SELECT
    r.run_id, r.project_id, r.transaction_id, r.cell_id, r.cell_revision,
    r.lease_owner_id, r.fence, r.lease_expires_at_ms,
    r.workspace_binding_digest, r.state, r.cell_state, w.state,
    r.source_digest, r.source_length, r.source_object_digest,
    r.baseline_checkpoint_id, r.result_checkpoint_id,
    r.environment_digest, r.kernel_generation, r.terminal_code,
    r.last_sequence, r.revision, r.created_at_ms, r.updated_at_ms,
    w.transaction_id, w.project_id, w.fence,
    w.baseline_checkpoint_id, w.result_checkpoint_id
FROM runs r
JOIN workspace_transactions w ON w.run_id = r.run_id
WHERE r.run_id = ?1";

struct RawRun {
    run_id: String,
    project_id: String,
    transaction_id: String,
    cell_id: String,
    cell_revision: i64,
    lease_owner_id: String,
    fence: i64,
    lease_expires_at_ms: i64,
    workspace_binding_digest: String,
    state: String,
    cell_state: String,
    transaction_state: String,
    source_digest: String,
    source_length: i64,
    source_object_digest: Option<String>,
    baseline_checkpoint_id: Option<String>,
    result_checkpoint_id: Option<String>,
    environment_digest: Option<String>,
    kernel_generation: Option<i64>,
    terminal_code: Option<String>,
    last_sequence: i64,
    revision: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    workspace_transaction_id: String,
    workspace_project_id: String,
    workspace_fence: i64,
    workspace_baseline_checkpoint_id: Option<String>,
    workspace_result_checkpoint_id: Option<String>,
}

fn map_raw_run(row: &Row<'_>) -> rusqlite::Result<RawRun> {
    Ok(RawRun {
        run_id: row.get(0)?,
        project_id: row.get(1)?,
        transaction_id: row.get(2)?,
        cell_id: row.get(3)?,
        cell_revision: row.get(4)?,
        lease_owner_id: row.get(5)?,
        fence: row.get(6)?,
        lease_expires_at_ms: row.get(7)?,
        workspace_binding_digest: row.get(8)?,
        state: row.get(9)?,
        cell_state: row.get(10)?,
        transaction_state: row.get(11)?,
        source_digest: row.get(12)?,
        source_length: row.get(13)?,
        source_object_digest: row.get(14)?,
        baseline_checkpoint_id: row.get(15)?,
        result_checkpoint_id: row.get(16)?,
        environment_digest: row.get(17)?,
        kernel_generation: row.get(18)?,
        terminal_code: row.get(19)?,
        last_sequence: row.get(20)?,
        revision: row.get(21)?,
        created_at_ms: row.get(22)?,
        updated_at_ms: row.get(23)?,
        workspace_transaction_id: row.get(24)?,
        workspace_project_id: row.get(25)?,
        workspace_fence: row.get(26)?,
        workspace_baseline_checkpoint_id: row.get(27)?,
        workspace_result_checkpoint_id: row.get(28)?,
    })
}

impl RawRun {
    fn parse(self) -> Result<RunRecord, RepositoryError> {
        let run_id = decode_storage(decode_run_key("run_id", &self.run_id))?;
        let project_id = decode_storage(decode_project_key("project_id", &self.project_id))?;
        let workspace_project_id =
            decode_storage(decode_project_key("project_id", &self.workspace_project_id))?;
        let transaction_id = decode_storage(decode_transaction_key(
            "transaction_id",
            &self.transaction_id,
        ))?;
        let workspace_transaction_id = decode_storage(decode_transaction_key(
            "transaction_id",
            &self.workspace_transaction_id,
        ))?;
        let owner_id = decode_storage(decode_lease_owner_key(
            "lease_owner_id",
            &self.lease_owner_id,
        ))?;
        let fence_value = decode_storage(decode_positive("fence", self.fence))?;
        let workspace_fence = decode_storage(decode_positive("fence", self.workspace_fence))?;
        let fence = FencingToken::new(fence_value).map_err(|_| RepositoryError::CorruptState)?;
        if project_id != workspace_project_id
            || transaction_id != workspace_transaction_id
            || fence_value != workspace_fence
        {
            return Err(RepositoryError::CorruptState);
        }
        let source_digest = decode_storage(decode_digest("source_digest", &self.source_digest))?;
        let source_object = self
            .source_object_digest
            .as_deref()
            .map(|value| decode_storage(decode_digest("source_object_digest", value)))
            .transpose()?;
        if source_object.is_some_and(|digest| digest != source_digest) {
            return Err(RepositoryError::CorruptState);
        }
        let state = decode_storage(RunState::decode("state", &self.state))?;
        let cell_state = decode_storage(CellState::decode("cell_state", &self.cell_state))?;
        let transaction_state = decode_storage(TransactionState::decode(
            "transaction_state",
            &self.transaction_state,
        ))?;
        if !run_shape_is_valid(state, cell_state, transaction_state) {
            return Err(RepositoryError::CorruptState);
        }
        let baseline = self
            .baseline_checkpoint_id
            .as_deref()
            .map(|value| decode_storage(decode_checkpoint_key("baseline_checkpoint_id", value)))
            .transpose()?;
        let workspace_baseline = self
            .workspace_baseline_checkpoint_id
            .as_deref()
            .map(|value| decode_storage(decode_checkpoint_key("baseline_checkpoint_id", value)))
            .transpose()?;
        let result = self
            .result_checkpoint_id
            .as_deref()
            .map(|value| decode_storage(decode_checkpoint_key("result_checkpoint_id", value)))
            .transpose()?;
        let workspace_result = self
            .workspace_result_checkpoint_id
            .as_deref()
            .map(|value| decode_storage(decode_checkpoint_key("result_checkpoint_id", value)))
            .transpose()?;
        if baseline != workspace_baseline || result != workspace_result {
            return Err(RepositoryError::CorruptState);
        }
        Ok(RunRecord {
            run_id,
            project_id,
            transaction_id,
            cell_id: decode_storage(decode_cell_key("cell_id", &self.cell_id))?,
            cell_revision: decode_storage(decode_positive("cell_revision", self.cell_revision))?,
            lease: LeaseGrant {
                project_id,
                owner_id,
                fence,
                expires_at_ms: decode_storage(decode_non_negative(
                    "lease_expires_at_ms",
                    self.lease_expires_at_ms,
                ))?,
            },
            workspace_binding: decode_storage(decode_digest(
                "workspace_binding_digest",
                &self.workspace_binding_digest,
            ))?,
            state,
            cell_state,
            transaction_state,
            source: BlobRef {
                digest: source_digest,
                length: decode_storage(decode_non_negative("source_length", self.source_length))?,
            },
            source_is_published: source_object.is_some(),
            baseline,
            result,
            environment: self
                .environment_digest
                .as_deref()
                .map(|value| decode_storage(decode_digest("environment_digest", value)))
                .transpose()?,
            kernel_generation: self
                .kernel_generation
                .map(|value| decode_storage(decode_positive("kernel_generation", value)))
                .transpose()?,
            terminal_code: self.terminal_code,
            last_sequence: decode_storage(decode_positive("last_sequence", self.last_sequence))?,
            revision: decode_storage(decode_positive("revision", self.revision))?,
            created_at_ms: decode_storage(decode_non_negative(
                "created_at_ms",
                self.created_at_ms,
            ))?,
            updated_at_ms: decode_storage(decode_non_negative(
                "updated_at_ms",
                self.updated_at_ms,
            ))?,
        })
    }
}

struct RawCheckpoint {
    manifest_digest: String,
    manifest_length: i64,
    backend: String,
    fidelity: String,
    git_context_digest: Option<String>,
    entry_count: i64,
    total_file_bytes: i64,
    created_at_ms: i64,
}

fn map_raw_checkpoint(row: &Row<'_>) -> rusqlite::Result<RawCheckpoint> {
    Ok(RawCheckpoint {
        manifest_digest: row.get(0)?,
        manifest_length: row.get(1)?,
        backend: row.get(2)?,
        fidelity: row.get(3)?,
        git_context_digest: row.get(4)?,
        entry_count: row.get(5)?,
        total_file_bytes: row.get(6)?,
        created_at_ms: row.get(7)?,
    })
}

struct RawCheckpointEntry {
    path: String,
    kind: String,
    object_digest: String,
    object_length: i64,
    is_executable: i64,
}

fn map_raw_checkpoint_entry(row: &Row<'_>) -> rusqlite::Result<RawCheckpointEntry> {
    Ok(RawCheckpointEntry {
        path: row.get(0)?,
        kind: row.get(1)?,
        object_digest: row.get(2)?,
        object_length: row.get(3)?,
        is_executable: row.get(4)?,
    })
}

impl RawCheckpoint {
    fn parse(
        self,
        id: CheckpointKey,
        raw_entries: Vec<RawCheckpointEntry>,
    ) -> Result<CheckpointRecord, RepositoryError> {
        let entry_count = decode_storage(decode_non_negative("entry_count", self.entry_count))?;
        let actual_count =
            u64::try_from(raw_entries.len()).map_err(|_| RepositoryError::NumericOverflow)?;
        if entry_count != actual_count {
            return Err(RepositoryError::CorruptState);
        }
        let entries = raw_entries
            .into_iter()
            .map(|raw| {
                Ok(CheckpointEntry {
                    path: raw.path,
                    kind: decode_storage(CheckpointEntryKind::decode("kind", &raw.kind))?,
                    object: BlobRef {
                        digest: decode_storage(decode_digest("object_digest", &raw.object_digest))?,
                        length: decode_storage(decode_non_negative(
                            "object_length",
                            raw.object_length,
                        ))?,
                    },
                    is_executable: decode_storage(decode_boolean(
                        "is_executable",
                        raw.is_executable,
                    ))?,
                })
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        Ok(CheckpointRecord {
            id,
            manifest: BlobRef {
                digest: decode_storage(decode_digest("manifest_digest", &self.manifest_digest))?,
                length: decode_storage(decode_non_negative(
                    "manifest_length",
                    self.manifest_length,
                ))?,
            },
            backend: decode_storage(CheckpointBackend::decode("backend", &self.backend))?,
            fidelity: decode_storage(RollbackFidelity::decode("fidelity", &self.fidelity))?,
            git_context: self
                .git_context_digest
                .as_deref()
                .map(|value| decode_storage(decode_digest("git_context_digest", value)))
                .transpose()?,
            entries,
            total_file_bytes: decode_storage(decode_non_negative(
                "total_file_bytes",
                self.total_file_bytes,
            ))?,
            created_at_ms: decode_storage(decode_non_negative(
                "created_at_ms",
                self.created_at_ms,
            ))?,
        })
    }
}

struct RawOutputReplay {
    sequence: i64,
    kind: String,
    event_stream: Option<String>,
    event_digest: Option<String>,
    event_length: Option<i64>,
    chunk_stream: Option<String>,
    chunk_digest: Option<String>,
    chunk_length: Option<i64>,
}

fn map_raw_output_replay(row: &Row<'_>) -> rusqlite::Result<RawOutputReplay> {
    Ok(RawOutputReplay {
        sequence: row.get(0)?,
        kind: row.get(1)?,
        event_stream: row.get(2)?,
        event_digest: row.get(3)?,
        event_length: row.get(4)?,
        chunk_stream: row.get(5)?,
        chunk_digest: row.get(6)?,
        chunk_length: row.get(7)?,
    })
}

impl RawOutputReplay {
    fn compare(self, stream: OutputStream, blob: BlobRef) -> OutputMutation {
        if self.sequence <= 0 || self.kind != "output" {
            return OutputMutation::CorruptState;
        }
        let Some(event_stream) = self.event_stream else {
            return OutputMutation::CorruptState;
        };
        let Some(event_digest) = self.event_digest else {
            return OutputMutation::CorruptState;
        };
        let Some(event_length) = self.event_length else {
            return OutputMutation::CorruptState;
        };
        let Some(chunk_stream) = self.chunk_stream else {
            return OutputMutation::CorruptState;
        };
        let Some(chunk_digest) = self.chunk_digest else {
            return OutputMutation::CorruptState;
        };
        let Some(chunk_length) = self.chunk_length else {
            return OutputMutation::CorruptState;
        };
        let Ok(event_stream) = OutputStream::decode("stream", &event_stream) else {
            return OutputMutation::CorruptState;
        };
        let Ok(event_digest) = decode_digest("blob_digest", &event_digest) else {
            return OutputMutation::CorruptState;
        };
        let Ok(event_length) = decode_non_negative("blob_length", event_length) else {
            return OutputMutation::CorruptState;
        };
        let Ok(chunk_stream) = OutputStream::decode("stream", &chunk_stream) else {
            return OutputMutation::CorruptState;
        };
        let Ok(chunk_digest) = decode_digest("blob_digest", &chunk_digest) else {
            return OutputMutation::CorruptState;
        };
        let Ok(chunk_length) = decode_non_negative("blob_length", chunk_length) else {
            return OutputMutation::CorruptState;
        };
        if event_stream != chunk_stream
            || event_digest != chunk_digest
            || event_length != chunk_length
        {
            return OutputMutation::CorruptState;
        }
        if event_stream == stream && event_digest == blob.digest && event_length == blob.length {
            OutputMutation::Applied(self.sequence)
        } else {
            OutputMutation::InvalidTransition
        }
    }
}

struct RawEvent {
    sequence: i64,
    kind: String,
    worker_sequence: Option<i64>,
    stream: Option<String>,
    blob_digest: Option<String>,
    blob_length: Option<i64>,
    occurred_at_ms: i64,
    chunk_stream: Option<String>,
    chunk_digest: Option<String>,
    chunk_length: Option<i64>,
}

fn map_raw_event(row: &Row<'_>) -> rusqlite::Result<RawEvent> {
    Ok(RawEvent {
        sequence: row.get(0)?,
        kind: row.get(1)?,
        worker_sequence: row.get(2)?,
        stream: row.get(3)?,
        blob_digest: row.get(4)?,
        blob_length: row.get(5)?,
        occurred_at_ms: row.get(6)?,
        chunk_stream: row.get(7)?,
        chunk_digest: row.get(8)?,
        chunk_length: row.get(9)?,
    })
}

impl RawEvent {
    fn parse(self) -> Result<StoredEvent, RepositoryError> {
        let sequence = decode_storage(decode_positive("sequence", self.sequence))?;
        let worker_sequence = self
            .worker_sequence
            .map(|value| decode_storage(decode_positive("worker_sequence", value)))
            .transpose()?;
        let stream = self
            .stream
            .as_deref()
            .map(|value| decode_storage(OutputStream::decode("stream", value)))
            .transpose()?;
        let blob = decode_storage(decode_optional_blob(
            "blob_digest",
            self.blob_digest.as_deref(),
            "blob_length",
            self.blob_length,
        ))?;
        let chunk_stream = self
            .chunk_stream
            .as_deref()
            .map(|value| decode_storage(OutputStream::decode("stream", value)))
            .transpose()?;
        let chunk_blob = decode_storage(decode_optional_blob(
            "blob_digest",
            self.chunk_digest.as_deref(),
            "blob_length",
            self.chunk_length,
        ))?;
        if self.kind == "output" {
            if worker_sequence.is_none()
                || stream.is_none()
                || blob.is_none()
                || chunk_stream != stream
                || chunk_blob != blob
            {
                return Err(RepositoryError::CorruptState);
            }
        } else if worker_sequence.is_some()
            || stream.is_some()
            || blob.is_some()
            || chunk_stream.is_some()
            || chunk_blob.is_some()
        {
            return Err(RepositoryError::CorruptState);
        }
        Ok(StoredEvent {
            sequence,
            kind: self.kind,
            worker_sequence,
            stream,
            blob,
            occurred_at_ms: decode_storage(decode_non_negative(
                "occurred_at_ms",
                self.occurred_at_ms,
            ))?,
        })
    }
}
