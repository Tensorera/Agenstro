use std::{num::NonZeroU16, path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use segno_core::{
    CronDialect, CronExpression, DispatchRequest, DstFoldPolicy, DstGapPolicy, FencingToken,
    IanaTimeZone, LeaseOwnerId, MisfirePolicy, OccurrenceId, OccurrenceState, OrchestrationRunId,
    OverlapPolicy, RetryPolicy, SchedulePolicy, ScheduleRevision, Sha256Digest, TaskId, UtcInstant,
};
use thiserror::Error;

const APPLICATION_ID: i64 = 0x5345_474E;
const SCHEMA_VERSION: i64 = 1;
const MAX_LIST_LIMIT: usize = 200;
const MAX_CLAIM_LIMIT: usize = 256;
const MAX_CANDIDATE_MULTIPLIER: usize = 4;

const MIGRATION_V1: &str = r#"
CREATE TABLE tasks (
    task_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    current_revision INTEGER NOT NULL CHECK (current_revision > 0),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE task_revisions (
    task_id TEXT NOT NULL REFERENCES tasks(task_id),
    revision INTEGER NOT NULL CHECK (revision > 0),
    package_digest TEXT NOT NULL,
    workflow_spec_digest TEXT NOT NULL,
    workflow_spec BLOB NOT NULL CHECK (length(workflow_spec) BETWEEN 1 AND 1048576),
    plan_digest TEXT,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (task_id, revision),
    UNIQUE (task_id, package_digest, revision)
);
CREATE TABLE schedules (
    task_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    cron_dialect TEXT NOT NULL,
    cron_expression TEXT NOT NULL,
    timezone TEXT NOT NULL,
    dst_gap TEXT NOT NULL,
    dst_fold TEXT NOT NULL,
    misfire_kind TEXT NOT NULL,
    misfire_value INTEGER NOT NULL,
    overlap_kind TEXT NOT NULL,
    overlap_limit INTEGER NOT NULL,
    retry_kind TEXT NOT NULL,
    retry_attempts INTEGER NOT NULL,
    retry_delay_ms INTEGER NOT NULL,
    jitter_ms INTEGER NOT NULL,
    next_fire_at_ms INTEGER NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    PRIMARY KEY (task_id, revision),
    FOREIGN KEY (task_id, revision) REFERENCES task_revisions(task_id, revision)
);
CREATE TABLE occurrences (
    occurrence_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    schedule_revision INTEGER NOT NULL,
    scheduled_for_ms INTEGER NOT NULL,
    not_before_ms INTEGER NOT NULL,
    state TEXT NOT NULL,
    orchestration_run_id TEXT,
    summary_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (task_id, schedule_revision, scheduled_for_ms),
    FOREIGN KEY (task_id, schedule_revision) REFERENCES task_revisions(task_id, revision)
);
CREATE INDEX occurrences_admission_idx
    ON occurrences(state, not_before_ms, occurrence_id);
CREATE INDEX occurrences_task_state_idx
    ON occurrences(task_id, state, occurrence_id);
CREATE TABLE leases (
    occurrence_id TEXT PRIMARY KEY REFERENCES occurrences(occurrence_id),
    owner TEXT NOT NULL,
    fencing_token INTEGER NOT NULL CHECK (fencing_token > 0),
    expires_at_ms INTEGER NOT NULL,
    heartbeat_at_ms INTEGER NOT NULL
);
CREATE TABLE dispatches (
    occurrence_id TEXT PRIMARY KEY REFERENCES occurrences(occurrence_id),
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    state TEXT NOT NULL,
    orchestration_run_id TEXT,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE outbox (
    outbox_id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurrence_id TEXT NOT NULL UNIQUE REFERENCES occurrences(occurrence_id),
    fencing_token INTEGER NOT NULL CHECK (fencing_token > 0),
    state TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX outbox_state_idx ON outbox(state, outbox_id);
CREATE TABLE events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurrence_id TEXT,
    kind TEXT NOT NULL,
    code TEXT,
    occurred_at_ms INTEGER NOT NULL
);
"#;

/// Immutable revision input prepared outside the SQLite transaction.
pub(crate) struct NewRevision {
    pub task_id: TaskId,
    pub name: String,
    pub package_digest: Sha256Digest,
    pub workflow_spec_digest: Sha256Digest,
    pub workflow_spec: Vec<u8>,
    pub policy: SchedulePolicy,
}

/// Stored revision and policy needed for compilation.
pub(crate) struct RevisionForCompile {
    pub task_id: TaskId,
    pub revision: ScheduleRevision,
    pub package_digest: Sha256Digest,
    pub workflow_spec_digest: Sha256Digest,
    pub workflow_spec: Vec<u8>,
    pub policy: SchedulePolicy,
}

/// Due schedule page item used by the bounded scheduler tick.
pub(crate) struct DueSchedule {
    pub task_id: TaskId,
    pub revision: ScheduleRevision,
    pub policy: SchedulePolicy,
    pub next_fire: UtcInstant,
}

/// One bounded task-list result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSummary {
    /// Stable task identifier.
    pub task_id: String,
    /// Current immutable revision.
    pub revision: u64,
    /// Whether the current revision is compiled and schedule-enabled.
    pub enabled: bool,
    /// Current package digest.
    pub package_digest: String,
    /// Frozen plan digest when enabled.
    pub plan_digest: Option<String>,
}

/// Stable cursor page of tasks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskListPage {
    /// Sorted page items.
    pub tasks: Vec<TaskSummary>,
    /// Last task ID when another page may be requested.
    pub next_after: Option<String>,
}

/// Bounded occurrence status without Tactus logs/artifacts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrenceStatus {
    /// Stable occurrence/idempotency key.
    pub occurrence_id: String,
    /// Owning task.
    pub task_id: String,
    /// Frozen revision.
    pub revision: u64,
    /// UTC scheduled instant in milliseconds.
    pub scheduled_for_ms: i64,
    /// Durable scheduler state.
    pub state: OccurrenceState,
    /// External orchestration reference when accepted.
    pub orchestration_run_id: Option<String>,
    /// Bounded terminal/recovery summary code.
    pub summary_code: Option<String>,
}

/// Single-owner Segno SQLite repository.
///
/// The value owns its connection and is intentionally not shared behind an
/// async pool. Application methods perform external calls only after repository
/// transactions return.
pub struct SqliteStore {
    connection: Connection,
}

impl SqliteStore {
    /// Opens, configures, checks, and migrates an absolute Segno database path.
    ///
    /// # Errors
    ///
    /// Rejects relative paths, a newer schema, failed integrity checks, or
    /// SQLite/filesystem failures.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if !path.is_absolute() {
            return Err(StoreError::DatabasePathNotAbsolute);
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(30))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;",
        )?;
        let application_id: i64 =
            connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
        if application_id != 0 && application_id != APPLICATION_ID {
            return Err(StoreError::WrongDatabase);
        }
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        let quick_check: String =
            connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        if quick_check != "ok" {
            return Err(StoreError::QuickCheckFailed);
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY CHECK (version > 0),
                name TEXT NOT NULL UNIQUE,
                checksum TEXT NOT NULL,
                applied_at_ms INTEGER NOT NULL
            );",
        )?;
        let current: Option<i64> =
            connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        if current.unwrap_or(0) > SCHEMA_VERSION {
            return Err(StoreError::FutureSchema);
        }
        if current.is_none() {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(MIGRATION_V1)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at_ms)
                 VALUES (1, 'segno_v1', 'segno-v1-20260801', 0)",
                [],
            )?;
            transaction.execute_batch("PRAGMA user_version = 1;")?;
            transaction.commit()?;
        } else {
            let migration: (String, String) = connection.query_row(
                "SELECT name, checksum FROM schema_migrations WHERE version = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if migration != ("segno_v1".to_owned(), "segno-v1-20260801".to_owned()) {
                return Err(StoreError::MigrationMismatch);
            }
        }
        Ok(Self { connection })
    }

    /// Inserts a new immutable revision and disables it until compilation.
    ///
    /// # Errors
    ///
    /// Returns a validation, arithmetic, or SQLite failure.
    pub(crate) fn import_revision(
        &mut self,
        input: NewRevision,
        now: UtcInstant,
    ) -> Result<ScheduleRevision, StoreError> {
        if input.name.is_empty() || input.name.len() > 120 || input.workflow_spec.is_empty() {
            return Err(StoreError::InvalidValue("revision"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<i64> = transaction
            .query_row(
                "SELECT current_revision FROM tasks WHERE task_id = ?1",
                [input.task_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let revision_i64 = previous
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(StoreError::RevisionExhausted)?;
        let revision_u64 =
            u64::try_from(revision_i64).map_err(|_| StoreError::RevisionExhausted)?;
        let revision =
            ScheduleRevision::new(revision_u64).map_err(|_| StoreError::RevisionExhausted)?;
        transaction.execute(
            "INSERT INTO tasks(task_id, name, current_revision, enabled, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 0, ?4, ?4)
             ON CONFLICT(task_id) DO UPDATE SET
                name = excluded.name,
                current_revision = excluded.current_revision,
                enabled = 0,
                updated_at_ms = excluded.updated_at_ms",
            params![input.task_id.as_str(), input.name, revision_i64, now.as_millis()],
        )?;
        transaction.execute(
            "INSERT INTO task_revisions(
                task_id, revision, package_digest, workflow_spec_digest,
                workflow_spec, plan_digest, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
            params![
                input.task_id.as_str(),
                revision_i64,
                input.package_digest.to_string(),
                input.workflow_spec_digest.to_string(),
                input.workflow_spec,
                now.as_millis(),
            ],
        )?;
        insert_schedule(
            &transaction,
            &input.task_id,
            revision,
            &input.policy,
            now,
            false,
        )?;
        transaction.commit()?;
        Ok(revision)
    }

    /// Loads one current immutable revision before the external compile call.
    ///
    /// # Errors
    ///
    /// Returns not-found/stale or typed decode/SQLite failures.
    pub(crate) fn revision_for_compile(
        &self,
        task_id: &TaskId,
        expected_revision: ScheduleRevision,
    ) -> Result<RevisionForCompile, StoreError> {
        self.connection
            .query_row(
                "SELECT r.package_digest, r.workflow_spec_digest, r.workflow_spec,
                        s.cron_dialect, s.cron_expression, s.timezone, s.dst_gap, s.dst_fold,
                        s.misfire_kind, s.misfire_value, s.overlap_kind, s.overlap_limit,
                        s.retry_kind, s.retry_attempts, s.retry_delay_ms, s.jitter_ms
                 FROM tasks t
                 JOIN task_revisions r ON r.task_id = t.task_id AND r.revision = t.current_revision
                 JOIN schedules s ON s.task_id = r.task_id AND s.revision = r.revision
                 WHERE t.task_id = ?1 AND t.current_revision = ?2",
                params![task_id.as_str(), to_i64(expected_revision.value())?],
                |row| {
                    let package: String = row.get(0)?;
                    let spec: String = row.get(1)?;
                    let workflow_spec = row.get(2)?;
                    let policy = decode_policy(row, 3)?;
                    Ok((package, spec, workflow_spec, policy))
                },
            )
            .optional()?
            .ok_or(StoreError::NotFound)
            .and_then(|(package, spec, workflow_spec, policy)| {
                Ok(RevisionForCompile {
                    task_id: task_id.clone(),
                    revision: expected_revision,
                    package_digest: Sha256Digest::parse(&package)
                        .map_err(|_| StoreError::CorruptValue("package digest"))?,
                    workflow_spec_digest: Sha256Digest::parse(&spec)
                        .map_err(|_| StoreError::CorruptValue("spec digest"))?,
                    workflow_spec,
                    policy,
                })
            })
    }

    /// Commits successful compilation and enables the exact expected revision.
    ///
    /// # Errors
    ///
    /// Rejects stale revision CAS or SQLite failure.
    pub(crate) fn enable_revision(
        &mut self,
        task_id: &TaskId,
        expected_revision: ScheduleRevision,
        plan_digest: Sha256Digest,
        next_fire: UtcInstant,
        now: UtcInstant,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE task_revisions SET plan_digest = ?3
             WHERE task_id = ?1 AND revision = ?2
               AND EXISTS (
                 SELECT 1 FROM tasks
                 WHERE tasks.task_id = task_revisions.task_id
                   AND tasks.current_revision = task_revisions.revision
               )",
            params![
                task_id.as_str(),
                to_i64(expected_revision.value())?,
                plan_digest.to_string(),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        transaction.execute(
            "UPDATE schedules SET enabled = 1, next_fire_at_ms = ?3
             WHERE task_id = ?1 AND revision = ?2",
            params![
                task_id.as_str(),
                to_i64(expected_revision.value())?,
                next_fire.as_millis()
            ],
        )?;
        transaction.execute(
            "UPDATE tasks SET enabled = 1, updated_at_ms = ?3
             WHERE task_id = ?1 AND current_revision = ?2",
            params![
                task_id.as_str(),
                to_i64(expected_revision.value())?,
                now.as_millis()
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Returns a stable bounded task page.
    ///
    /// # Errors
    ///
    /// Rejects a zero/excessive limit or returns a typed decode/SQLite failure.
    pub fn list_tasks(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<TaskListPage, StoreError> {
        if limit == 0 || limit > MAX_LIST_LIMIT {
            return Err(StoreError::InvalidLimit);
        }
        let mut statement = self.connection.prepare(
            "SELECT t.task_id, t.current_revision, t.enabled, r.package_digest, r.plan_digest
             FROM tasks t
             JOIN task_revisions r ON r.task_id = t.task_id AND r.revision = t.current_revision
             WHERE t.task_id > ?1
             ORDER BY t.task_id
             LIMIT ?2",
        )?;
        let fetch = limit.saturating_add(1);
        let rows =
            statement.query_map(params![after.unwrap_or(""), to_i64(fetch as u64)?], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
        let mut tasks = rows
            .map(|row| {
                let (task_id, revision, enabled, package_digest, plan_digest) = row?;
                let revision = ScheduleRevision::new(
                    u64::try_from(revision).map_err(|_| StoreError::CorruptValue("revision"))?,
                )
                .map_err(|_| StoreError::CorruptValue("revision"))?;
                Ok(TaskSummary {
                    task_id,
                    revision: revision.value(),
                    enabled: enabled != 0,
                    package_digest,
                    plan_digest,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let has_more = tasks.len() > limit;
        tasks.truncate(limit);
        let next_after = has_more
            .then(|| tasks.last().map(|task| task.task_id.clone()))
            .flatten();
        Ok(TaskListPage { tasks, next_after })
    }

    /// Creates or returns the unique manual occurrence for a compiled revision.
    ///
    /// # Errors
    ///
    /// Returns not-found/uncompiled, digest, conversion, or SQLite failures.
    pub fn create_manual_occurrence(
        &mut self,
        task_id: &TaskId,
        scheduled_for: UtcInstant,
        now: UtcInstant,
    ) -> Result<OccurrenceId, StoreError> {
        let revision_i64: i64 = self
            .connection
            .query_row(
                "SELECT t.current_revision
                 FROM tasks t JOIN task_revisions r
                   ON r.task_id = t.task_id AND r.revision = t.current_revision
                 WHERE t.task_id = ?1 AND r.plan_digest IS NOT NULL",
                [task_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let revision = ScheduleRevision::new(
            u64::try_from(revision_i64).map_err(|_| StoreError::CorruptValue("revision"))?,
        )
        .map_err(|_| StoreError::CorruptValue("revision"))?;
        self.insert_occurrence(task_id, revision, scheduled_for, scheduled_for, now)
    }

    fn insert_occurrence(
        &mut self,
        task_id: &TaskId,
        revision: ScheduleRevision,
        scheduled_for: UtcInstant,
        not_before: UtcInstant,
        now: UtcInstant,
    ) -> Result<OccurrenceId, StoreError> {
        let id = OccurrenceId::derive(task_id, revision, scheduled_for)
            .map_err(|_| StoreError::CorruptValue("occurrence id"))?;
        self.connection.execute(
            "INSERT OR IGNORE INTO occurrences(
                occurrence_id, task_id, schedule_revision, scheduled_for_ms,
                not_before_ms, state, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?6)",
            params![
                id.as_str(),
                task_id.as_str(),
                to_i64(revision.value())?,
                scheduled_for.as_millis(),
                not_before.as_millis(),
                now.as_millis(),
            ],
        )?;
        Ok(id)
    }

    pub(crate) fn due_schedules(
        &self,
        now: UtcInstant,
        limit: usize,
    ) -> Result<Vec<DueSchedule>, StoreError> {
        if limit == 0 || limit > MAX_LIST_LIMIT {
            return Err(StoreError::InvalidLimit);
        }
        let mut statement = self.connection.prepare(
            "SELECT s.task_id, s.revision, s.next_fire_at_ms,
                    s.cron_dialect, s.cron_expression, s.timezone, s.dst_gap, s.dst_fold,
                    s.misfire_kind, s.misfire_value, s.overlap_kind, s.overlap_limit,
                    s.retry_kind, s.retry_attempts, s.retry_delay_ms, s.jitter_ms
             FROM schedules s
             JOIN tasks t ON t.task_id = s.task_id AND t.current_revision = s.revision
             WHERE s.enabled = 1 AND t.enabled = 1 AND s.next_fire_at_ms <= ?1
             ORDER BY s.next_fire_at_ms, s.task_id
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![now.as_millis(), to_i64(limit as u64)?], |row| {
            let task: String = row.get(0)?;
            let revision: i64 = row.get(1)?;
            let next_fire: i64 = row.get(2)?;
            let policy = decode_policy(row, 3)?;
            Ok((task, revision, next_fire, policy))
        })?;
        rows.map(|row| {
            let (task, revision, next_fire, policy) = row?;
            Ok(DueSchedule {
                task_id: TaskId::parse(&task).map_err(|_| StoreError::CorruptValue("task id"))?,
                revision: ScheduleRevision::new(
                    u64::try_from(revision).map_err(|_| StoreError::CorruptValue("revision"))?,
                )
                .map_err(|_| StoreError::CorruptValue("revision"))?,
                policy,
                next_fire: UtcInstant::from_millis(next_fire),
            })
        })
        .collect()
    }

    pub(crate) fn advance_schedule(
        &mut self,
        schedule: &DueSchedule,
        selected: &[UtcInstant],
        next_fire: UtcInstant,
        now: UtcInstant,
    ) -> Result<Vec<OccurrenceId>, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut ids = Vec::with_capacity(selected.len());
        for scheduled_for in selected {
            let id = OccurrenceId::derive(&schedule.task_id, schedule.revision, *scheduled_for)
                .map_err(|_| StoreError::CorruptValue("occurrence id"))?;
            let jitter = deterministic_jitter(&id, schedule.policy.jitter_ms);
            let jitter_i64 =
                i64::try_from(jitter).map_err(|_| StoreError::InvalidValue("jitter"))?;
            let not_before = scheduled_for
                .as_millis()
                .checked_add(jitter_i64)
                .ok_or(StoreError::InvalidValue("jitter"))?;
            transaction.execute(
                "INSERT OR IGNORE INTO occurrences(
                    occurrence_id, task_id, schedule_revision, scheduled_for_ms,
                    not_before_ms, state, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?6)",
                params![
                    id.as_str(),
                    schedule.task_id.as_str(),
                    to_i64(schedule.revision.value())?,
                    scheduled_for.as_millis(),
                    not_before,
                    now.as_millis(),
                ],
            )?;
            ids.push(id);
        }
        let changed = transaction.execute(
            "UPDATE schedules SET next_fire_at_ms = ?4
             WHERE task_id = ?1 AND revision = ?2 AND next_fire_at_ms = ?3 AND enabled = 1",
            params![
                schedule.task_id.as_str(),
                to_i64(schedule.revision.value())?,
                schedule.next_fire.as_millis(),
                next_fire.as_millis(),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        transaction.commit()?;
        Ok(ids)
    }

    /// Claims at most the caller's currently available dispatch capacity.
    ///
    /// Every accepted claim increments a fence and creates/refreshes a durable
    /// outbox intent in the same short transaction.
    ///
    /// # Errors
    ///
    /// Rejects invalid limits/TTL or returns typed decode/SQLite failures.
    pub fn claim_due(
        &mut self,
        owner: &LeaseOwnerId,
        now: UtcInstant,
        ttl_ms: u64,
        capacity: usize,
    ) -> Result<Vec<DispatchRequest>, StoreError> {
        self.claim(owner, now, ttl_ms, capacity, "queued")
    }

    /// Reclaims bounded uncertain dispatches after lease expiry/restart.
    ///
    /// # Errors
    ///
    /// Rejects invalid limits/TTL or returns typed decode/SQLite failures.
    pub fn claim_reconciliation(
        &mut self,
        owner: &LeaseOwnerId,
        now: UtcInstant,
        ttl_ms: u64,
        capacity: usize,
    ) -> Result<Vec<DispatchRequest>, StoreError> {
        self.claim(owner, now, ttl_ms, capacity, "dispatching")
    }

    fn claim(
        &mut self,
        owner: &LeaseOwnerId,
        now: UtcInstant,
        ttl_ms: u64,
        capacity: usize,
        state: &'static str,
    ) -> Result<Vec<DispatchRequest>, StoreError> {
        if capacity == 0 || capacity > MAX_CLAIM_LIMIT || ttl_ms == 0 || ttl_ms > i64::MAX as u64 {
            return Err(StoreError::InvalidLimit);
        }
        let expires = now
            .as_millis()
            .checked_add(i64::try_from(ttl_ms).map_err(|_| StoreError::InvalidLimit)?)
            .ok_or(StoreError::InvalidLimit)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scan_limit = capacity
            .saturating_mul(MAX_CANDIDATE_MULTIPLIER)
            .min(MAX_CLAIM_LIMIT);
        let candidates = load_candidates(&transaction, now, state, scan_limit)?;
        let mut claims = Vec::with_capacity(capacity);
        for candidate in candidates {
            if claims.len() >= capacity {
                break;
            }
            if state == "queued" && !overlap_allows(&transaction, &candidate)? {
                continue;
            }
            let existing: Option<(String, i64, i64)> = transaction
                .query_row(
                    "SELECT owner, fencing_token, expires_at_ms FROM leases WHERE occurrence_id = ?1",
                    [candidate.occurrence_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if existing
                .as_ref()
                .is_some_and(|(current_owner, _, expires_at)| {
                    *expires_at > now.as_millis() && current_owner != owner.as_str()
                })
            {
                continue;
            }
            let token_i64 = existing.map_or(Ok(1_i64), |(_, token, _)| {
                token.checked_add(1).ok_or(StoreError::FenceExhausted)
            })?;
            transaction.execute(
                "INSERT INTO leases(occurrence_id, owner, fencing_token, expires_at_ms, heartbeat_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(occurrence_id) DO UPDATE SET
                   owner = excluded.owner,
                   fencing_token = excluded.fencing_token,
                   expires_at_ms = excluded.expires_at_ms,
                   heartbeat_at_ms = excluded.heartbeat_at_ms",
                params![candidate.occurrence_id.as_str(), owner.as_str(), token_i64, expires, now.as_millis()],
            )?;
            transaction.execute(
                "UPDATE occurrences SET state = 'dispatching', updated_at_ms = ?2
                 WHERE occurrence_id = ?1",
                params![candidate.occurrence_id.as_str(), now.as_millis()],
            )?;
            transaction.execute(
                "INSERT INTO outbox(occurrence_id, fencing_token, state, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, 'pending', ?3, ?3)
                 ON CONFLICT(occurrence_id) DO UPDATE SET
                   fencing_token = excluded.fencing_token,
                   state = 'pending',
                   updated_at_ms = excluded.updated_at_ms",
                params![candidate.occurrence_id.as_str(), token_i64, now.as_millis()],
            )?;
            let token = FencingToken::new(
                u64::try_from(token_i64).map_err(|_| StoreError::FenceExhausted)?,
            )
            .map_err(|_| StoreError::FenceExhausted)?;
            claims.push(DispatchRequest {
                occurrence_id: candidate.occurrence_id,
                revision: candidate.revision,
                plan_digest: candidate.plan_digest,
                owner: owner.clone(),
                fencing_token: token,
            });
        }
        transaction.commit()?;
        Ok(claims)
    }

    /// Commits an accepted orchestration reference only under the current fence.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::FenceRejected`] for a stale owner/token.
    pub fn record_dispatch(
        &mut self,
        request: &DispatchRequest,
        run_id: &OrchestrationRunId,
        now: UtcInstant,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE occurrences
             SET state = 'dispatched', orchestration_run_id = ?4, updated_at_ms = ?5
             WHERE occurrence_id = ?1 AND state = 'dispatching'
               AND EXISTS (
                 SELECT 1 FROM leases
                 WHERE leases.occurrence_id = occurrences.occurrence_id
                   AND leases.owner = ?2 AND leases.fencing_token = ?3
                   AND leases.expires_at_ms > ?5
               )",
            params![
                request.occurrence_id.as_str(),
                request.owner.as_str(),
                to_i64(request.fencing_token.value())?,
                run_id.as_str(),
                now.as_millis(),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::FenceRejected);
        }
        transaction.execute(
            "INSERT INTO dispatches(occurrence_id, attempt, state, orchestration_run_id, updated_at_ms)
             VALUES (?1, 1, 'accepted', ?2, ?3)
             ON CONFLICT(occurrence_id) DO UPDATE SET
               state = 'accepted', orchestration_run_id = excluded.orchestration_run_id,
               updated_at_ms = excluded.updated_at_ms",
            params![request.occurrence_id.as_str(), run_id.as_str(), now.as_millis()],
        )?;
        transaction.execute(
            "UPDATE outbox SET state = 'sent', updated_at_ms = ?2
             WHERE occurrence_id = ?1 AND fencing_token = ?3",
            params![
                request.occurrence_id.as_str(),
                now.as_millis(),
                to_i64(request.fencing_token.value())?,
            ],
        )?;
        transaction.execute(
            "INSERT INTO events(occurrence_id, kind, code, occurred_at_ms)
             VALUES (?1, 'dispatch_accepted', NULL, ?2)",
            params![request.occurrence_id.as_str(), now.as_millis()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Records a known pre-accept rejection under the current fence.
    ///
    /// The scheduler does not create a second logical workflow implicitly.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::FenceRejected`] for a stale owner/token or a typed
    /// validation/SQLite failure.
    pub fn record_dispatch_failure(
        &mut self,
        request: &DispatchRequest,
        code: &str,
        now: UtcInstant,
    ) -> Result<(), StoreError> {
        if code.is_empty() || code.len() > 128 || code.chars().any(char::is_control) {
            return Err(StoreError::InvalidValue("dispatch failure code"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE occurrences
             SET state = 'recovery_required', summary_code = ?4, updated_at_ms = ?5
             WHERE occurrence_id = ?1 AND state = 'dispatching'
               AND EXISTS (
                 SELECT 1 FROM leases
                 WHERE leases.occurrence_id = occurrences.occurrence_id
                   AND leases.owner = ?2 AND leases.fencing_token = ?3
                   AND leases.expires_at_ms > ?5
               )",
            params![
                request.occurrence_id.as_str(),
                request.owner.as_str(),
                to_i64(request.fencing_token.value())?,
                code,
                now.as_millis(),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::FenceRejected);
        }
        transaction.execute(
            "INSERT INTO dispatches(occurrence_id, attempt, state, orchestration_run_id, updated_at_ms)
             VALUES (?1, 1, 'rejected', NULL, ?2)
             ON CONFLICT(occurrence_id) DO UPDATE SET
               state = 'rejected', updated_at_ms = excluded.updated_at_ms",
            params![request.occurrence_id.as_str(), now.as_millis()],
        )?;
        transaction.execute(
            "UPDATE outbox SET state = 'failed', updated_at_ms = ?2
             WHERE occurrence_id = ?1 AND fencing_token = ?3",
            params![
                request.occurrence_id.as_str(),
                now.as_millis(),
                to_i64(request.fencing_token.value())?,
            ],
        )?;
        transaction.execute(
            "INSERT INTO events(occurrence_id, kind, code, occurred_at_ms)
             VALUES (?1, 'dispatch_rejected', ?2, ?3)",
            params![request.occurrence_id.as_str(), code, now.as_millis()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Returns one bounded occurrence status.
    ///
    /// # Errors
    ///
    /// Returns not found, corrupt state codec, or SQLite failure.
    pub fn occurrence_status(&self, id: &OccurrenceId) -> Result<OccurrenceStatus, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT occurrence_id, task_id, schedule_revision, scheduled_for_ms,
                        state, orchestration_run_id, summary_code
                 FROM occurrences WHERE occurrence_id = ?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        Ok(OccurrenceStatus {
            occurrence_id: row.0,
            task_id: row.1,
            revision: u64::try_from(row.2).map_err(|_| StoreError::CorruptValue("revision"))?,
            scheduled_for_ms: row.3,
            state: decode_occurrence_state(&row.4)
                .ok_or(StoreError::CorruptValue("occurrence state"))?,
            orchestration_run_id: row.5,
            summary_code: row.6,
        })
    }

    /// Records a bounded terminal orchestration summary and releases overlap.
    ///
    /// Segno stores no workflow log or artifact payload. Duplicate delivery of
    /// the same terminal state is idempotent; a conflicting terminal state is
    /// rejected.
    ///
    /// # Errors
    ///
    /// Returns not found/state conflict, invalid code, or SQLite failure.
    pub fn record_terminal_summary(
        &mut self,
        id: &OccurrenceId,
        succeeded: bool,
        code: &str,
        now: UtcInstant,
    ) -> Result<(), StoreError> {
        if code.is_empty() || code.len() > 128 || code.chars().any(char::is_control) {
            return Err(StoreError::InvalidValue("terminal summary code"));
        }
        let target = if succeeded { "succeeded" } else { "failed" };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT state FROM occurrences WHERE occurrence_id = ?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let current = current.ok_or(StoreError::NotFound)?;
        if current == target {
            transaction.commit()?;
            return Ok(());
        }
        if current != "dispatched" {
            return Err(StoreError::StateConflict);
        }
        transaction.execute(
            "UPDATE occurrences SET state = ?2, summary_code = ?3, updated_at_ms = ?4
             WHERE occurrence_id = ?1 AND state = 'dispatched'",
            params![id.as_str(), target, code, now.as_millis()],
        )?;
        transaction.execute("DELETE FROM leases WHERE occurrence_id = ?1", [id.as_str()])?;
        transaction.execute(
            "INSERT INTO events(occurrence_id, kind, code, occurred_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![id.as_str(), target, code, now.as_millis()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Performs a bounded passive WAL checkpoint before owner shutdown.
    ///
    /// # Errors
    ///
    /// Returns a SQLite failure; it never waits without SQLite's busy timeout.
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        self.connection
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))?;
        Ok(())
    }
}

struct Candidate {
    occurrence_id: OccurrenceId,
    task_id: TaskId,
    revision: ScheduleRevision,
    plan_digest: Sha256Digest,
    overlap: OverlapPolicy,
}

fn load_candidates(
    transaction: &Transaction<'_>,
    now: UtcInstant,
    state: &str,
    limit: usize,
) -> Result<Vec<Candidate>, StoreError> {
    let mut statement = transaction.prepare(
        "SELECT o.occurrence_id, o.task_id, o.schedule_revision, r.plan_digest,
                s.overlap_kind, s.overlap_limit
         FROM occurrences o
         JOIN task_revisions r ON r.task_id = o.task_id AND r.revision = o.schedule_revision
         JOIN schedules s ON s.task_id = o.task_id AND s.revision = o.schedule_revision
         WHERE o.state = ?1 AND o.not_before_ms <= ?2 AND r.plan_digest IS NOT NULL
           AND (?1 != 'dispatching' OR EXISTS (
             SELECT 1 FROM outbox x
             WHERE x.occurrence_id = o.occurrence_id AND x.state = 'pending'
           ))
         ORDER BY o.not_before_ms, o.occurrence_id
         LIMIT ?3",
    )?;
    let rows = statement.query_map(
        params![state, now.as_millis(), to_i64(limit as u64)?],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    rows.map(|row| {
        let (occurrence, task, revision, plan, overlap, overlap_limit) = row?;
        Ok(Candidate {
            occurrence_id: OccurrenceId::parse(&occurrence)
                .map_err(|_| StoreError::CorruptValue("occurrence id"))?,
            task_id: TaskId::parse(&task).map_err(|_| StoreError::CorruptValue("task id"))?,
            revision: ScheduleRevision::new(
                u64::try_from(revision).map_err(|_| StoreError::CorruptValue("revision"))?,
            )
            .map_err(|_| StoreError::CorruptValue("revision"))?,
            plan_digest: Sha256Digest::parse(&plan)
                .map_err(|_| StoreError::CorruptValue("plan digest"))?,
            overlap: decode_overlap(&overlap, overlap_limit)?,
        })
    })
    .collect()
}

fn overlap_allows(
    transaction: &Transaction<'_>,
    candidate: &Candidate,
) -> Result<bool, StoreError> {
    let active: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM occurrences
         WHERE task_id = ?1 AND state IN ('dispatching', 'dispatched')",
        [candidate.task_id.as_str()],
        |row| row.get(0),
    )?;
    Ok(match candidate.overlap {
        OverlapPolicy::Forbid | OverlapPolicy::QueueOne => active == 0,
        OverlapPolicy::AllowWithLimit(limit) => active < i64::from(limit.get()),
    })
}

fn insert_schedule(
    transaction: &Transaction<'_>,
    task_id: &TaskId,
    revision: ScheduleRevision,
    policy: &SchedulePolicy,
    now: UtcInstant,
    enabled: bool,
) -> Result<(), StoreError> {
    let (misfire_kind, misfire_value) = encode_misfire(policy.misfire);
    let (overlap_kind, overlap_limit) = encode_overlap(policy.overlap);
    let (retry_kind, retry_attempts, retry_delay) = encode_retry(policy.retry);
    transaction.execute(
        "INSERT INTO schedules(
            task_id, revision, cron_dialect, cron_expression, timezone, dst_gap, dst_fold,
            misfire_kind, misfire_value, overlap_kind, overlap_limit,
            retry_kind, retry_attempts, retry_delay_ms, jitter_ms, next_fire_at_ms, enabled
         ) VALUES (?1, ?2, 'unix5', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            task_id.as_str(),
            to_i64(revision.value())?,
            policy.cron.as_str(),
            policy.timezone.as_str(),
            encode_gap(policy.dst_gap),
            encode_fold(policy.dst_fold),
            misfire_kind,
            misfire_value,
            overlap_kind,
            overlap_limit,
            retry_kind,
            retry_attempts,
            retry_delay,
            to_i64(policy.jitter_ms)?,
            now.as_millis(),
            if enabled { 1_i64 } else { 0_i64 },
        ],
    )?;
    Ok(())
}

fn decode_policy(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<SchedulePolicy> {
    let dialect: String = row.get(offset)?;
    let cron: String = row.get(offset + 1)?;
    let timezone: String = row.get(offset + 2)?;
    let gap: String = row.get(offset + 3)?;
    let fold: String = row.get(offset + 4)?;
    let misfire_kind: String = row.get(offset + 5)?;
    let misfire_value: i64 = row.get(offset + 6)?;
    let overlap_kind: String = row.get(offset + 7)?;
    let overlap_limit: i64 = row.get(offset + 8)?;
    let retry_kind: String = row.get(offset + 9)?;
    let retry_attempts: i64 = row.get(offset + 10)?;
    let retry_delay: i64 = row.get(offset + 11)?;
    let jitter: i64 = row.get(offset + 12)?;
    decode_policy_values(
        &dialect,
        &cron,
        &timezone,
        &gap,
        &fold,
        &misfire_kind,
        misfire_value,
        &overlap_kind,
        overlap_limit,
        &retry_kind,
        retry_attempts,
        retry_delay,
        jitter,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)
}

#[allow(clippy::too_many_arguments)]
fn decode_policy_values(
    dialect: &str,
    cron: &str,
    timezone: &str,
    gap: &str,
    fold: &str,
    misfire_kind: &str,
    misfire_value: i64,
    overlap_kind: &str,
    overlap_limit: i64,
    retry_kind: &str,
    retry_attempts: i64,
    retry_delay: i64,
    jitter: i64,
) -> Result<SchedulePolicy, StoreError> {
    if dialect != "unix5" {
        return Err(StoreError::CorruptValue("cron dialect"));
    }
    let dst_gap = match gap {
        "skip" => DstGapPolicy::Skip,
        "next_valid" => DstGapPolicy::NextValid,
        _ => return Err(StoreError::CorruptValue("DST gap")),
    };
    let dst_fold = match fold {
        "first" => DstFoldPolicy::First,
        "second" => DstFoldPolicy::Second,
        "both" => DstFoldPolicy::Both,
        _ => return Err(StoreError::CorruptValue("DST fold")),
    };
    let misfire = match misfire_kind {
        "skip" => MisfirePolicy::Skip {
            grace_ms: u64::try_from(misfire_value)
                .map_err(|_| StoreError::CorruptValue("misfire"))?,
        },
        "coalesce" => MisfirePolicy::Coalesce,
        "catch_up" => MisfirePolicy::BoundedCatchUp(nonzero_u16(misfire_value, "misfire")?),
        _ => return Err(StoreError::CorruptValue("misfire")),
    };
    let overlap = decode_overlap(overlap_kind, overlap_limit)?;
    let retry = match retry_kind {
        "none" => RetryPolicy::None,
        "bounded_idempotent" => RetryPolicy::BoundedIdempotent {
            max_attempts: nonzero_u16(retry_attempts, "retry")?,
            delay_ms: u64::try_from(retry_delay).map_err(|_| StoreError::CorruptValue("retry"))?,
        },
        _ => return Err(StoreError::CorruptValue("retry")),
    };
    Ok(SchedulePolicy {
        dialect: CronDialect::UnixFiveField,
        cron: CronExpression::parse(cron).map_err(|_| StoreError::CorruptValue("cron"))?,
        timezone: IanaTimeZone::parse(timezone)
            .map_err(|_| StoreError::CorruptValue("timezone"))?,
        dst_gap,
        dst_fold,
        misfire,
        overlap,
        retry,
        jitter_ms: u64::try_from(jitter).map_err(|_| StoreError::CorruptValue("jitter"))?,
    })
}

fn encode_gap(value: DstGapPolicy) -> &'static str {
    match value {
        DstGapPolicy::Skip => "skip",
        DstGapPolicy::NextValid => "next_valid",
    }
}

fn encode_fold(value: DstFoldPolicy) -> &'static str {
    match value {
        DstFoldPolicy::First => "first",
        DstFoldPolicy::Second => "second",
        DstFoldPolicy::Both => "both",
    }
}

fn encode_misfire(value: MisfirePolicy) -> (&'static str, i64) {
    match value {
        MisfirePolicy::Skip { grace_ms } => ("skip", i64::try_from(grace_ms).unwrap_or(i64::MAX)),
        MisfirePolicy::Coalesce => ("coalesce", 0),
        MisfirePolicy::BoundedCatchUp(limit) => ("catch_up", i64::from(limit.get())),
    }
}

fn encode_overlap(value: OverlapPolicy) -> (&'static str, i64) {
    match value {
        OverlapPolicy::Forbid => ("forbid", 1),
        OverlapPolicy::QueueOne => ("queue_one", 1),
        OverlapPolicy::AllowWithLimit(limit) => ("allow_with_limit", i64::from(limit.get())),
    }
}

fn decode_overlap(kind: &str, limit: i64) -> Result<OverlapPolicy, StoreError> {
    match kind {
        "forbid" => Ok(OverlapPolicy::Forbid),
        "queue_one" => Ok(OverlapPolicy::QueueOne),
        "allow_with_limit" => Ok(OverlapPolicy::AllowWithLimit(nonzero_u16(
            limit, "overlap",
        )?)),
        _ => Err(StoreError::CorruptValue("overlap")),
    }
}

fn encode_retry(value: RetryPolicy) -> (&'static str, i64, i64) {
    match value {
        RetryPolicy::None => ("none", 0, 0),
        RetryPolicy::BoundedIdempotent {
            max_attempts,
            delay_ms,
        } => (
            "bounded_idempotent",
            i64::from(max_attempts.get()),
            i64::try_from(delay_ms).unwrap_or(i64::MAX),
        ),
    }
}

fn nonzero_u16(value: i64, field: &'static str) -> Result<NonZeroU16, StoreError> {
    u16::try_from(value)
        .ok()
        .and_then(NonZeroU16::new)
        .ok_or(StoreError::CorruptValue(field))
}

fn decode_occurrence_state(value: &str) -> Option<OccurrenceState> {
    match value {
        "queued" => Some(OccurrenceState::Queued),
        "dispatching" => Some(OccurrenceState::Dispatching),
        "dispatched" => Some(OccurrenceState::Dispatched),
        "succeeded" => Some(OccurrenceState::Succeeded),
        "failed" => Some(OccurrenceState::Failed),
        "recovery_required" => Some(OccurrenceState::RecoveryRequired),
        "skipped" => Some(OccurrenceState::Skipped),
        _ => None,
    }
}

fn deterministic_jitter(id: &OccurrenceId, maximum: u64) -> u64 {
    if maximum == 0 {
        return 0;
    }
    let digest = Sha256Digest::parse(id.as_str().trim_start_matches("occ-")).ok();
    let mut bytes = [0_u8; 8];
    if let Some(digest) = digest {
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
    }
    u64::from_be_bytes(bytes) % maximum.saturating_add(1)
}

fn to_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::InvalidValue("integer"))
}

/// Segno repository failure.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Database path is not absolute.
    #[error("Segno database path must be absolute")]
    DatabasePathNotAbsolute,
    /// File belongs to another application.
    #[error("SQLite application_id does not belong to Segno")]
    WrongDatabase,
    /// Database quick check did not return `ok`.
    #[error("SQLite quick check failed")]
    QuickCheckFailed,
    /// Binary refuses a newer schema.
    #[error("Segno database schema is newer than this binary")]
    FutureSchema,
    /// Applied migration name/checksum changed.
    #[error("Segno database migration definition does not match")]
    MigrationMismatch,
    /// Entity was not found.
    #[error("Segno record was not found")]
    NotFound,
    /// Expected revision no longer matches current revision.
    #[error("task or schedule revision conflict")]
    RevisionConflict,
    /// Occurrence state does not permit the requested transition.
    #[error("occurrence state conflict")]
    StateConflict,
    /// Revision sequence overflowed.
    #[error("task revision sequence is exhausted")]
    RevisionExhausted,
    /// Fence sequence overflowed.
    #[error("lease fencing sequence is exhausted")]
    FenceExhausted,
    /// Old lease owner/token attempted to write.
    #[error("stale fencing token was rejected")]
    FenceRejected,
    /// Page, claim, or TTL bound is invalid.
    #[error("bounded repository limit is invalid")]
    InvalidLimit,
    /// Caller value violates a persistence contract.
    #[error("repository value is invalid: {0}")]
    InvalidValue(&'static str),
    /// Persisted value violates its typed codec.
    #[error("persisted repository value is corrupt: {0}")]
    CorruptValue(&'static str),
    /// SQLite operation failed.
    #[error("Segno SQLite operation failed")]
    Sqlite(#[from] rusqlite::Error),
}
