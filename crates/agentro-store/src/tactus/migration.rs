use crate::{Migration, MigrationDefinitionError};

/// Frozen name of the original Tactus runtime migration.
pub const TACTUS_V1_MIGRATION_NAME: &str = "create_tactus_runtime";

/// Frozen SQL bytes of the original Tactus runtime migration.
pub const TACTUS_V1_SCHEMA_SQL: &str = r#"
CREATE TABLE projects (
    project_id TEXT PRIMARY KEY,
    last_fence INTEGER NOT NULL CHECK (last_fence >= 0)
);

CREATE TABLE project_leases (
    project_id TEXT PRIMARY KEY REFERENCES projects(project_id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL,
    fence INTEGER NOT NULL CHECK (fence > 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms >= 0)
);

CREATE TABLE cells (
    cell_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision > 0),
    source_digest TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE TABLE checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    manifest_digest TEXT NOT NULL,
    manifest_length INTEGER NOT NULL CHECK (manifest_length >= 0),
    backend TEXT NOT NULL CHECK (backend IN ('non_git', 'git_aware')),
    fidelity TEXT NOT NULL CHECK (fidelity IN ('full_manifest', 'declared_paths')),
    git_context_digest TEXT,
    entry_count INTEGER NOT NULL CHECK (entry_count >= 0),
    total_file_bytes INTEGER NOT NULL CHECK (total_file_bytes >= 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0)
);

CREATE TABLE checkpoint_entries (
    checkpoint_id TEXT NOT NULL REFERENCES checkpoints(checkpoint_id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('file', 'symlink')),
    object_digest TEXT NOT NULL,
    object_length INTEGER NOT NULL CHECK (object_length >= 0),
    is_executable INTEGER NOT NULL CHECK (is_executable IN (0, 1)),
    PRIMARY KEY (checkpoint_id, path)
);

CREATE TABLE runs (
    run_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE,
    request_digest TEXT NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    transaction_id TEXT NOT NULL UNIQUE,
    cell_id TEXT NOT NULL REFERENCES cells(cell_id),
    cell_revision INTEGER NOT NULL CHECK (cell_revision > 0),
    lease_owner_id TEXT NOT NULL,
    fence INTEGER NOT NULL CHECK (fence > 0),
    lease_expires_at_ms INTEGER NOT NULL CHECK (lease_expires_at_ms >= 0),
    workspace_binding_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK (
        state IN ('pending', 'running', 'cancelling', 'recovering',
                  'succeeded', 'failed', 'cancelled', 'interrupted')
    ),
    cell_state TEXT NOT NULL CHECK (
        cell_state IN ('queued', 'running', 'recovering',
                       'succeeded', 'failed', 'cancelled', 'interrupted')
    ),
    source_digest TEXT NOT NULL,
    source_length INTEGER NOT NULL CHECK (source_length >= 0),
    source_object_digest TEXT,
    baseline_checkpoint_id TEXT REFERENCES checkpoints(checkpoint_id),
    result_checkpoint_id TEXT REFERENCES checkpoints(checkpoint_id),
    environment_digest TEXT,
    kernel_generation INTEGER CHECK (kernel_generation > 0),
    terminal_code TEXT,
    last_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_sequence >= 0),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE TABLE workspace_transactions (
    transaction_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES runs(run_id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(project_id),
    fence INTEGER NOT NULL CHECK (fence > 0),
    state TEXT NOT NULL CHECK (
        state IN ('prepared', 'active', 'committed', 'abandoned', 'conflict')
    ),
    baseline_checkpoint_id TEXT REFERENCES checkpoints(checkpoint_id),
    result_checkpoint_id TEXT REFERENCES checkpoints(checkpoint_id),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE TABLE events (
    run_id TEXT NOT NULL REFERENCES runs(run_id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    kind TEXT NOT NULL,
    worker_sequence INTEGER CHECK (worker_sequence > 0),
    stream TEXT CHECK (stream IN ('stdout', 'stderr', 'display')),
    blob_digest TEXT,
    blob_length INTEGER CHECK (blob_length >= 0),
    occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
    PRIMARY KEY (run_id, sequence),
    UNIQUE (run_id, worker_sequence)
);

CREATE TABLE output_chunks (
    run_id TEXT NOT NULL,
    event_sequence INTEGER NOT NULL,
    stream TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr', 'display')),
    blob_digest TEXT NOT NULL,
    blob_length INTEGER NOT NULL CHECK (blob_length >= 0),
    PRIMARY KEY (run_id, event_sequence),
    FOREIGN KEY (run_id, event_sequence) REFERENCES events(run_id, sequence) ON DELETE CASCADE
);

CREATE INDEX runs_project_created_idx
    ON runs(project_id, created_at_ms, run_id);
CREATE INDEX events_run_sequence_idx
    ON events(run_id, sequence);
CREATE INDEX checkpoint_entries_checkpoint_path_idx
    ON checkpoint_entries(checkpoint_id, path);
"#;

/// Builds the immutable Tactus v1 migration definition.
///
/// # Errors
///
/// Returns [`MigrationDefinitionError`] if the frozen definition violates the
/// shared migration bounds.
pub fn tactus_v1_migration() -> Result<Migration, MigrationDefinitionError> {
    Migration::new(1, TACTUS_V1_MIGRATION_NAME, TACTUS_V1_SCHEMA_SQL)
}
