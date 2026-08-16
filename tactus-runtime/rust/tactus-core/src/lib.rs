//! Durable Tactus execution domain and application composition.
//!
//! Tactus owns stable cell identity, fenced project execution, workspace
//! transactions, external-CAS checkpoints, bounded output references, worker
//! lifecycle validation, and crash reconciliation. Python remains an external
//! worker: neither interpreter memory nor notebooks are durable authority.
//!
//! The crate is transport independent. `tactusd` wire handlers convert the
//! generated Agentro Protobuf API to the [`TactusDaemon`] commands exported
//! here; provider-specific and Python implementation types do not enter this
//! boundary.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod checkpoint;
mod daemon;
mod id;
mod process;
mod script;
mod state;
mod store;
mod worker;

use agentro_contracts::{DaemonInstanceId, DaemonKind, Product, ServerIdentity};

pub use agentro_contracts::{ErrorCode, RequestId, Sha256Digest};

pub use checkpoint::{
    BlobRef, CasCheckpointBackend, Checkpoint, CheckpointBackendKind, CheckpointConfig,
    CheckpointEntry, CheckpointEntryKind, CheckpointError, CheckpointId, GitCliMetadata,
    GitMetadataPort, PathPolicy, RestoreConflict, RestoreConflictReason, RestoreReport,
    RollbackFidelity, ScanBudget, WorkspacePort,
};
pub use daemon::{
    BeginRequest, Clock, DaemonConfig, ExecuteRequest, ManualClock, RunEvent, RunEventKind,
    RunSnapshot, StateDurability, SystemClock, TactusDaemon, TactusError, WatchPage,
};
pub use id::{
    CellId, FencingToken, LeaseOwnerId, ProjectId, RunId, TactusIdError, WorkspaceTransactionId,
};
pub use process::{
    CancellationSource, CancellationToken, ContainmentKind, NativeProcessSupervisor, OutputBudget,
    ProcessCapabilities, ProcessError, ProcessOutput, ProcessSpec, ProcessSupervisor,
    ProcessTimeouts, TerminationReason,
};
pub use script::{
    MAX_CELL_TITLE_BYTES, MAX_SCRIPT_BYTES, MAX_SCRIPT_CELLS, NormalizedScript, ScriptCell,
    ScriptCellKind, ScriptError, normalize_script, parse_script,
};
pub use state::{
    CellState, CellTransition, RunState, RunTransition, StateTransitionError, TransactionState,
    TransactionTransition,
};
pub use worker::{
    FramedWorker, FramedWorkerConfig, OutputStream, WorkerCommand, WorkerCompletion, WorkerError,
    WorkerEvent, WorkerEventSink, WorkerPayloadDecoder, WorkerPort, WorkerTerminal,
    encode_worker_frame,
};

/// The unchanged Tactus distribution identity.
pub const PRODUCT: Product = Product::TactusRuntime;
/// The daemon that will own execution and workspace transactions.
pub const DAEMON: DaemonKind = DaemonKind::Tactusd;

/// Creates a type-safe Tactus daemon identity for bootstrap metadata.
#[must_use]
pub const fn server_identity(instance_id: DaemonInstanceId) -> ServerIdentity {
    ServerIdentity::new(DAEMON, instance_id)
}

#[cfg(test)]
mod tests {
    use agentro_contracts::DaemonInstanceId;

    use super::{PRODUCT, server_identity};

    #[test]
    fn preserves_tactus_distribution_identity() {
        let identity = server_identity(DaemonInstanceId::generate());

        assert_eq!(PRODUCT.distribution_name(), "tactus-runtime");
        assert_eq!(identity.product(), PRODUCT);
        assert_eq!(identity.daemon().executable_name(), "tactusd");
    }
}
