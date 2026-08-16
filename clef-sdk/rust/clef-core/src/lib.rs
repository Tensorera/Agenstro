//! Deterministic, transport-independent Clef workflow domain core.
//!
//! The crate owns versioned workflow values, pure DAG compilation, bounded
//! ready-set scheduling, attempt transitions, and the artifact publish gate.
//! It performs no I/O and has no Tokio, storage, gRPC, or provider dependency.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod attempt;
mod compiler;
mod identifier;
mod model;
mod publish;
mod scheduler;

use agentro_contracts::{DaemonInstanceId, DaemonKind, Product, ServerIdentity};

pub use attempt::{Attempt, AttemptCommand, AttemptNumber, AttemptState, TransitionError};
pub use compiler::{
    CompileContext, CompileError, CompileIssueCode, CompiledTask, ExecutionPlan, ReadySet,
    ValidationIssue, ValidationReport, compile_workflow,
};
pub use identifier::{
    ArtifactName, DomainFunctionName, IdentifierError, ProjectPath, RunId, TaskId, WorkflowId,
};
pub use model::{
    ArtifactBinding, ArtifactKind, ArtifactSpec, EffectKind, EffectPolicy, EffectRule, Effort,
    MAX_ARTIFACTS_PER_TASK, MAX_BINDINGS, MAX_TASKS, ModelError, REASONING_EFFORT_CAPABILITY,
    SchemaVersion, TaskSpec, WorkflowPolicy, WorkflowSpec,
};
pub use publish::{
    ProducedArtifact, PublishDecision, PublishGate, PublishRejection, PublishRequest,
    RequiredOutputsGate,
};
pub use scheduler::{
    ReadyScheduler, SchedulerError, TaskOutcome, TaskScheduleState, WorkflowScheduleState,
};

/// The unchanged Clef distribution identity.
pub const PRODUCT: Product = Product::ClefSdk;
/// The daemon that owns Clef workflow and normalized agent orchestration.
pub const DAEMON: DaemonKind = DaemonKind::Agentrod;

/// Creates a type-safe Clef daemon identity for bootstrap metadata.
#[must_use]
pub const fn server_identity(instance_id: DaemonInstanceId) -> ServerIdentity {
    ServerIdentity::new(DAEMON, instance_id)
}

#[cfg(test)]
mod tests {
    use agentro_contracts::DaemonInstanceId;

    use super::{PRODUCT, server_identity};

    #[test]
    fn preserves_clef_distribution_identity() {
        let identity = server_identity(DaemonInstanceId::generate());

        assert_eq!(PRODUCT.distribution_name(), "clef-sdk");
        assert_eq!(identity.product(), PRODUCT);
        assert_eq!(identity.daemon().executable_name(), "agentrod");
    }
}
