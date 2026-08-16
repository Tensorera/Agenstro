use std::{error::Error, path::PathBuf, time::Duration};

use agentro_contracts::{RequestId, Sha256Digest};
use agentro_store::{
    StoreConfig,
    tactus::{
        model as storage,
        repository::{
            Repository as StorageRepository, RepositoryError as StorageRepositoryError,
            RepositoryOwner as StorageRepositoryOwner,
        },
    },
};
use thiserror::Error;

use crate::{
    BlobRef, CellId, CellState, Checkpoint, CheckpointBackendKind, CheckpointEntry,
    CheckpointEntryKind, CheckpointId, FencingToken, LeaseOwnerId, OutputStream, ProjectId,
    RollbackFidelity, RunId, RunState, TransactionState, WorkspaceTransactionId,
};

pub(crate) use agentro_store::JournalMode;

#[derive(Debug, Error)]
pub(crate) enum RepositoryError {
    #[error("run was not found")]
    NotFound,
    #[error("request ID was reused with a different payload")]
    IdempotencyConflict,
    #[error("project already has a live lease")]
    LeaseConflict,
    #[error("project fencing token is stale or expired")]
    FenceRejected,
    #[error("run state does not allow the requested operation")]
    InvalidTransition,
    #[error("workspace binding does not match the durable run")]
    WorkspaceBindingMismatch,
    #[error("durable Tactus state is corrupt or from an unsupported schema")]
    CorruptState,
    #[error("durable integer value is outside SQLite range")]
    NumericOverflow,
    #[error("durable output budget would be exceeded")]
    OutputBudgetExceeded,
    #[error("Tactus storage repository failed")]
    Storage(#[source] Box<dyn Error + Send + Sync>),
}

pub(crate) struct RepositoryOwner {
    owner: StorageRepositoryOwner,
    repository: Repository,
}

impl RepositoryOwner {
    pub(crate) fn open(
        database: PathBuf,
        queue_capacity: usize,
        busy_timeout: Duration,
        journal_mode: JournalMode,
        startup_timeout: Duration,
    ) -> Result<Self, RepositoryError> {
        let config = StoreConfig::new(queue_capacity, busy_timeout, journal_mode)
            .map_err(|error| RepositoryError::Storage(Box::new(error)))?;
        let owner = StorageRepositoryOwner::open(database, config, startup_timeout)
            .map_err(map_storage_error)?;
        let repository = Repository {
            inner: owner.repository(),
        };
        Ok(Self { owner, repository })
    }

    pub(crate) fn repository(&self) -> Repository {
        self.repository.clone()
    }

    pub(crate) fn shutdown(&mut self, timeout: Duration) -> Result<(), RepositoryError> {
        self.owner.shutdown(timeout).map_err(map_storage_error)
    }
}

#[derive(Clone)]
pub(crate) struct Repository {
    inner: StorageRepository,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LeaseGrant {
    pub(crate) project_id: ProjectId,
    pub(crate) owner_id: LeaseOwnerId,
    pub(crate) fence: FencingToken,
    pub(crate) expires_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct BeginIntent {
    pub(crate) request_id: RequestId,
    pub(crate) request_digest: Sha256Digest,
    pub(crate) run_id: RunId,
    pub(crate) transaction_id: WorkspaceTransactionId,
    pub(crate) project_id: ProjectId,
    pub(crate) cell_id: CellId,
    pub(crate) source: BlobRef,
    pub(crate) workspace_binding: Sha256Digest,
    pub(crate) owner_id: LeaseOwnerId,
    pub(crate) now_ms: u64,
    pub(crate) expires_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct BeginIntentResult {
    pub(crate) run: RunRecord,
    pub(crate) lease: LeaseGrant,
    pub(crate) replayed: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RunRecord {
    pub(crate) run_id: RunId,
    pub(crate) project_id: ProjectId,
    pub(crate) transaction_id: WorkspaceTransactionId,
    pub(crate) cell_id: CellId,
    pub(crate) cell_revision: u64,
    pub(crate) lease: LeaseGrant,
    pub(crate) workspace_binding: Sha256Digest,
    pub(crate) state: RunState,
    pub(crate) cell_state: CellState,
    pub(crate) transaction_state: TransactionState,
    pub(crate) source: BlobRef,
    pub(crate) source_is_published: bool,
    pub(crate) baseline: Option<CheckpointId>,
    pub(crate) result: Option<CheckpointId>,
    pub(crate) environment: Option<Sha256Digest>,
    pub(crate) kernel_generation: Option<u64>,
    pub(crate) terminal_code: Option<String>,
    pub(crate) last_sequence: u64,
    pub(crate) revision: u64,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct StoredEvent {
    pub(crate) sequence: u64,
    pub(crate) kind: String,
    pub(crate) worker_sequence: Option<u64>,
    pub(crate) stream: Option<OutputStream>,
    pub(crate) blob: Option<BlobRef>,
    pub(crate) occurred_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FinishDisposition {
    Failed,
    Cancelled,
    Interrupted,
    Conflict,
}

impl Repository {
    pub(crate) fn begin_intent(
        &self,
        input: BeginIntent,
    ) -> Result<BeginIntentResult, RepositoryError> {
        let result = self
            .inner
            .begin_intent(storage::BeginIntent {
                request_id: input.request_id,
                request_digest: input.request_digest,
                run_id: run_key(input.run_id)?,
                transaction_id: transaction_key(input.transaction_id)?,
                project_id: project_key(input.project_id)?,
                cell_id: cell_key(input.cell_id)?,
                source: blob_to_storage(input.source),
                workspace_binding: input.workspace_binding,
                owner_id: lease_owner_key(input.owner_id)?,
                now_ms: input.now_ms,
                expires_at_ms: input.expires_at_ms,
            })
            .map_err(map_storage_error)?;
        Ok(BeginIntentResult {
            run: run_from_storage(result.run)?,
            lease: lease_from_storage(result.lease)?,
            replayed: result.replayed,
        })
    }

    pub(crate) fn activate(
        &self,
        run_id: RunId,
        lease: LeaseGrant,
        source: BlobRef,
        baseline: &Checkpoint,
        now_ms: u64,
    ) -> Result<RunRecord, RepositoryError> {
        let checkpoint = checkpoint_to_storage(baseline, now_ms)?;
        self.inner
            .activate(
                run_key(run_id)?,
                lease_to_storage(lease)?,
                blob_to_storage(source),
                &checkpoint,
                now_ms,
            )
            .map_err(map_storage_error)
            .and_then(run_from_storage)
    }

    pub(crate) fn start_execution(
        &self,
        run_id: RunId,
        lease: LeaseGrant,
        workspace_binding: Sha256Digest,
        now_ms: u64,
        execution_expires_at_ms: u64,
    ) -> Result<RunRecord, RepositoryError> {
        self.inner
            .start_execution(
                run_key(run_id)?,
                lease_to_storage(lease)?,
                workspace_binding,
                now_ms,
                execution_expires_at_ms,
            )
            .map_err(map_storage_error)
            .and_then(run_from_storage)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn append_output(
        &self,
        run_id: RunId,
        lease: LeaseGrant,
        worker_sequence: u64,
        stream: OutputStream,
        blob: BlobRef,
        now_ms: u64,
        max_bytes: u64,
        max_records: u64,
    ) -> Result<u64, RepositoryError> {
        let budget = storage::OutputBudget::new(max_bytes, max_records)
            .map_err(|_| RepositoryError::InvalidTransition)?;
        self.inner
            .append_output(storage::AppendOutput {
                run_id: run_key(run_id)?,
                lease: lease_to_storage(lease)?,
                worker_sequence,
                stream: output_stream_to_storage(stream)?,
                blob: blob_to_storage(blob),
                now_ms,
                budget,
            })
            .map_err(map_storage_error)
    }

    pub(crate) fn request_cancel(
        &self,
        run_id: RunId,
        now_ms: u64,
    ) -> Result<(RunRecord, bool), RepositoryError> {
        let run_key = run_key(run_id)?;
        match self.inner.request_cancel(run_key, now_ms) {
            Ok((run, should_signal)) => Ok((run_from_storage(run)?, should_signal)),
            Err(StorageRepositoryError::InvalidTransition) => {
                let current = self.inner.run(run_key).map_err(map_storage_error)?;
                let current = run_from_storage(current)?;
                if current.state.is_terminal() {
                    Ok((current, false))
                } else {
                    Err(RepositoryError::InvalidTransition)
                }
            }
            Err(error) => Err(map_storage_error(error)),
        }
    }

    pub(crate) fn finish_success(
        &self,
        run_id: RunId,
        lease: LeaseGrant,
        result: &Checkpoint,
        environment: Sha256Digest,
        kernel_generation: u64,
        now_ms: u64,
    ) -> Result<RunRecord, RepositoryError> {
        self.inner
            .finish_success(storage::FinishSuccess {
                run_id: run_key(run_id)?,
                lease: lease_to_storage(lease)?,
                result: checkpoint_to_storage(result, now_ms)?,
                environment,
                kernel_generation,
                now_ms,
            })
            .map_err(map_storage_error)
            .and_then(run_from_storage)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_terminal(
        &self,
        run_id: RunId,
        lease: LeaseGrant,
        disposition: FinishDisposition,
        code: &str,
        environment: Option<Sha256Digest>,
        kernel_generation: Option<u64>,
        now_ms: u64,
    ) -> Result<RunRecord, RepositoryError> {
        self.inner
            .finish_terminal(storage::FinishTerminal {
                run_id: run_key(run_id)?,
                lease: lease_to_storage(lease)?,
                disposition: finish_disposition_to_storage(disposition),
                code: code.to_owned(),
                environment,
                kernel_generation,
                now_ms,
            })
            .map_err(map_storage_error)
            .and_then(run_from_storage)
    }

    pub(crate) fn run(&self, run_id: RunId) -> Result<RunRecord, RepositoryError> {
        self.inner
            .run(run_key(run_id)?)
            .map_err(map_storage_error)
            .and_then(run_from_storage)
    }

    pub(crate) fn watch(
        &self,
        run_id: RunId,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Vec<StoredEvent>, RepositoryError> {
        self.inner
            .watch(run_key(run_id)?, after_sequence, limit)
            .map_err(map_storage_error)?
            .into_iter()
            .map(event_from_storage)
            .collect()
    }

    pub(crate) fn checkpoint(
        &self,
        checkpoint_id: CheckpointId,
    ) -> Result<Checkpoint, RepositoryError> {
        self.inner
            .checkpoint(checkpoint_key(checkpoint_id)?)
            .map_err(map_storage_error)
            .and_then(checkpoint_from_storage)
    }

    pub(crate) fn reconcile_incomplete(&self, now_ms: u64) -> Result<u64, RepositoryError> {
        self.inner
            .reconcile_incomplete(now_ms)
            .map_err(map_storage_error)
    }
}

fn map_storage_error(error: StorageRepositoryError) -> RepositoryError {
    match error {
        StorageRepositoryError::NotFound => RepositoryError::NotFound,
        StorageRepositoryError::IdempotencyConflict => RepositoryError::IdempotencyConflict,
        StorageRepositoryError::LeaseConflict => RepositoryError::LeaseConflict,
        StorageRepositoryError::FenceRejected => RepositoryError::FenceRejected,
        StorageRepositoryError::InvalidTransition => RepositoryError::InvalidTransition,
        StorageRepositoryError::WorkspaceBindingMismatch => {
            RepositoryError::WorkspaceBindingMismatch
        }
        StorageRepositoryError::CorruptState => RepositoryError::CorruptState,
        StorageRepositoryError::NumericOverflow => RepositoryError::NumericOverflow,
        StorageRepositoryError::OutputBudgetExceeded => RepositoryError::OutputBudgetExceeded,
        other => RepositoryError::Storage(Box::new(other)),
    }
}

fn fail_closed<T, E>(result: Result<T, E>) -> Result<T, RepositoryError> {
    result.map_err(|_| RepositoryError::CorruptState)
}

fn project_key(value: ProjectId) -> Result<storage::ProjectKey, RepositoryError> {
    fail_closed(storage::ProjectKey::parse(&value.to_string()))
}

fn project_id(value: storage::ProjectKey) -> Result<ProjectId, RepositoryError> {
    fail_closed(ProjectId::parse(&value.to_string()))
}

fn cell_key(value: CellId) -> Result<storage::CellKey, RepositoryError> {
    fail_closed(storage::CellKey::parse(&value.to_string()))
}

fn cell_id(value: storage::CellKey) -> Result<CellId, RepositoryError> {
    fail_closed(CellId::parse(&value.to_string()))
}

fn run_key(value: RunId) -> Result<storage::RunKey, RepositoryError> {
    fail_closed(storage::RunKey::parse(&value.to_string()))
}

fn run_id(value: storage::RunKey) -> Result<RunId, RepositoryError> {
    fail_closed(RunId::parse(&value.to_string()))
}

fn transaction_key(
    value: WorkspaceTransactionId,
) -> Result<storage::WorkspaceTransactionKey, RepositoryError> {
    fail_closed(storage::WorkspaceTransactionKey::parse(&value.to_string()))
}

fn transaction_id(
    value: storage::WorkspaceTransactionKey,
) -> Result<WorkspaceTransactionId, RepositoryError> {
    fail_closed(WorkspaceTransactionId::parse(&value.to_string()))
}

fn lease_owner_key(value: LeaseOwnerId) -> Result<storage::LeaseOwnerKey, RepositoryError> {
    fail_closed(storage::LeaseOwnerKey::parse(&value.to_string()))
}

fn lease_owner_id(value: storage::LeaseOwnerKey) -> Result<LeaseOwnerId, RepositoryError> {
    fail_closed(LeaseOwnerId::parse(&value.to_string()))
}

fn checkpoint_key(value: CheckpointId) -> Result<storage::CheckpointKey, RepositoryError> {
    fail_closed(storage::CheckpointKey::parse(&value.to_string()))
}

fn checkpoint_id(value: storage::CheckpointKey) -> Result<CheckpointId, RepositoryError> {
    fail_closed(CheckpointId::parse(&value.to_string()))
}

fn fence_to_storage(value: FencingToken) -> Result<storage::FencingToken, RepositoryError> {
    fail_closed(storage::FencingToken::new(value.value()))
}

fn fence_from_storage(value: storage::FencingToken) -> Result<FencingToken, RepositoryError> {
    fail_closed(FencingToken::new(value.value()))
}

fn lease_to_storage(value: LeaseGrant) -> Result<storage::LeaseGrant, RepositoryError> {
    Ok(storage::LeaseGrant {
        project_id: project_key(value.project_id)?,
        owner_id: lease_owner_key(value.owner_id)?,
        fence: fence_to_storage(value.fence)?,
        expires_at_ms: value.expires_at_ms,
    })
}

fn lease_from_storage(value: storage::LeaseGrant) -> Result<LeaseGrant, RepositoryError> {
    Ok(LeaseGrant {
        project_id: project_id(value.project_id)?,
        owner_id: lease_owner_id(value.owner_id)?,
        fence: fence_from_storage(value.fence)?,
        expires_at_ms: value.expires_at_ms,
    })
}

const fn blob_to_storage(value: BlobRef) -> storage::BlobRef {
    storage::BlobRef {
        digest: value.digest(),
        length: value.length(),
    }
}

const fn blob_from_storage(value: storage::BlobRef) -> BlobRef {
    BlobRef::new(value.digest, value.length)
}

fn run_state_to_storage(value: RunState) -> Result<storage::RunState, RepositoryError> {
    fail_closed(storage::RunState::decode("state", value.as_str()))
}

fn run_state_from_storage(value: storage::RunState) -> Result<RunState, RepositoryError> {
    let converted = RunState::parse(value.as_str()).ok_or(RepositoryError::CorruptState)?;
    if run_state_to_storage(converted)? == value {
        Ok(converted)
    } else {
        Err(RepositoryError::CorruptState)
    }
}

fn cell_state_to_storage(value: CellState) -> Result<storage::CellState, RepositoryError> {
    fail_closed(storage::CellState::decode("cell_state", value.as_str()))
}

fn cell_state_from_storage(value: storage::CellState) -> Result<CellState, RepositoryError> {
    let converted = CellState::parse(value.as_str()).ok_or(RepositoryError::CorruptState)?;
    if cell_state_to_storage(converted)? == value {
        Ok(converted)
    } else {
        Err(RepositoryError::CorruptState)
    }
}

fn transaction_state_to_storage(
    value: TransactionState,
) -> Result<storage::TransactionState, RepositoryError> {
    fail_closed(storage::TransactionState::decode(
        "transaction_state",
        value.as_str(),
    ))
}

fn transaction_state_from_storage(
    value: storage::TransactionState,
) -> Result<TransactionState, RepositoryError> {
    let converted = TransactionState::parse(value.as_str()).ok_or(RepositoryError::CorruptState)?;
    if transaction_state_to_storage(converted)? == value {
        Ok(converted)
    } else {
        Err(RepositoryError::CorruptState)
    }
}

fn output_stream_to_storage(value: OutputStream) -> Result<storage::OutputStream, RepositoryError> {
    fail_closed(storage::OutputStream::decode("stream", value.as_str()))
}

fn output_stream_from_storage(
    value: storage::OutputStream,
) -> Result<OutputStream, RepositoryError> {
    OutputStream::parse(value.as_str()).ok_or(RepositoryError::CorruptState)
}

fn checkpoint_backend_to_storage(
    value: CheckpointBackendKind,
) -> Result<storage::CheckpointBackend, RepositoryError> {
    fail_closed(storage::CheckpointBackend::decode(
        "backend",
        value.as_str(),
    ))
}

fn checkpoint_backend_from_storage(
    value: storage::CheckpointBackend,
) -> Result<CheckpointBackendKind, RepositoryError> {
    CheckpointBackendKind::parse(value.as_str()).ok_or(RepositoryError::CorruptState)
}

fn fidelity_to_storage(
    value: RollbackFidelity,
) -> Result<storage::RollbackFidelity, RepositoryError> {
    fail_closed(storage::RollbackFidelity::decode(
        "fidelity",
        value.as_str(),
    ))
}

fn fidelity_from_storage(
    value: storage::RollbackFidelity,
) -> Result<RollbackFidelity, RepositoryError> {
    RollbackFidelity::parse(value.as_str()).ok_or(RepositoryError::CorruptState)
}

fn entry_kind_to_storage(
    value: CheckpointEntryKind,
) -> Result<storage::CheckpointEntryKind, RepositoryError> {
    fail_closed(storage::CheckpointEntryKind::decode("kind", value.as_str()))
}

fn entry_kind_from_storage(
    value: storage::CheckpointEntryKind,
) -> Result<CheckpointEntryKind, RepositoryError> {
    CheckpointEntryKind::parse(value.as_str()).ok_or(RepositoryError::CorruptState)
}

const fn finish_disposition_to_storage(value: FinishDisposition) -> storage::FinishDisposition {
    match value {
        FinishDisposition::Failed => storage::FinishDisposition::Failed,
        FinishDisposition::Cancelled => storage::FinishDisposition::Cancelled,
        FinishDisposition::Interrupted => storage::FinishDisposition::Interrupted,
        FinishDisposition::Conflict => storage::FinishDisposition::Conflict,
    }
}

fn checkpoint_to_storage(
    value: &Checkpoint,
    created_at_ms: u64,
) -> Result<storage::CheckpointRecord, RepositoryError> {
    let entries = value
        .entries()
        .iter()
        .map(|entry| {
            Ok(storage::CheckpointEntry {
                path: entry.path().to_owned(),
                kind: entry_kind_to_storage(entry.kind())?,
                object: blob_to_storage(entry.object()),
                is_executable: entry.is_executable(),
            })
        })
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    Ok(storage::CheckpointRecord {
        id: checkpoint_key(value.id())?,
        manifest: blob_to_storage(value.manifest()),
        backend: checkpoint_backend_to_storage(value.backend())?,
        fidelity: fidelity_to_storage(value.fidelity())?,
        git_context: value.git_context(),
        entries,
        total_file_bytes: value.total_file_bytes(),
        created_at_ms,
    })
}

fn checkpoint_from_storage(
    value: storage::CheckpointRecord,
) -> Result<Checkpoint, RepositoryError> {
    let entries = value
        .entries
        .into_iter()
        .map(|entry| {
            Ok(CheckpointEntry::from_stored(
                entry.path,
                entry_kind_from_storage(entry.kind)?,
                blob_from_storage(entry.object),
                entry.is_executable,
            ))
        })
        .collect::<Result<Vec<_>, RepositoryError>>()?;
    Ok(Checkpoint::from_stored(
        checkpoint_id(value.id)?,
        blob_from_storage(value.manifest),
        checkpoint_backend_from_storage(value.backend)?,
        fidelity_from_storage(value.fidelity)?,
        value.git_context,
        entries,
        value.total_file_bytes,
    ))
}

fn run_from_storage(value: storage::RunRecord) -> Result<RunRecord, RepositoryError> {
    Ok(RunRecord {
        run_id: run_id(value.run_id)?,
        project_id: project_id(value.project_id)?,
        transaction_id: transaction_id(value.transaction_id)?,
        cell_id: cell_id(value.cell_id)?,
        cell_revision: value.cell_revision,
        lease: lease_from_storage(value.lease)?,
        workspace_binding: value.workspace_binding,
        state: run_state_from_storage(value.state)?,
        cell_state: cell_state_from_storage(value.cell_state)?,
        transaction_state: transaction_state_from_storage(value.transaction_state)?,
        source: blob_from_storage(value.source),
        source_is_published: value.source_is_published,
        baseline: value.baseline.map(checkpoint_id).transpose()?,
        result: value.result.map(checkpoint_id).transpose()?,
        environment: value.environment,
        kernel_generation: value.kernel_generation,
        terminal_code: value.terminal_code,
        last_sequence: value.last_sequence,
        revision: value.revision,
        created_at_ms: value.created_at_ms,
        updated_at_ms: value.updated_at_ms,
    })
}

fn event_from_storage(value: storage::StoredEvent) -> Result<StoredEvent, RepositoryError> {
    Ok(StoredEvent {
        sequence: value.sequence,
        kind: value.kind,
        worker_sequence: value.worker_sequence,
        stream: value.stream.map(output_stream_from_storage).transpose()?,
        blob: value.blob.map(blob_from_storage),
        occurred_at_ms: value.occurred_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_adapter_values_round_trip() -> Result<(), RepositoryError> {
        let project = ProjectId::generate();
        let owner = LeaseOwnerId::generate();
        let lease = LeaseGrant {
            project_id: project,
            owner_id: owner,
            fence: FencingToken::new(7).map_err(|_| RepositoryError::CorruptState)?,
            expires_at_ms: 99,
        };

        let restored = lease_from_storage(lease_to_storage(lease)?)?;
        assert_eq!(restored.project_id, project);
        assert_eq!(restored.owner_id, owner);
        assert_eq!(restored.fence.value(), 7);
        assert_eq!(restored.expires_at_ms, 99);
        assert_eq!(
            run_state_from_storage(run_state_to_storage(RunState::Recovering)?)?,
            RunState::Recovering
        );
        assert_eq!(
            output_stream_from_storage(output_stream_to_storage(OutputStream::Display)?)?,
            OutputStream::Display
        );
        Ok(())
    }
}
