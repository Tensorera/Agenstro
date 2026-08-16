//! Durable Segno scheduling domain and external service ports.
//!
//! Segno owns immutable task revisions, civil-time policy, persistent
//! occurrence identity, lease fencing, bounded dispatch admission, and the
//! idempotent hand-off to `agentrod`. It never executes package stages and it
//! never owns Tactus logs or artifacts.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod occurrence;
mod policy;
mod port;

use agentro_contracts::{DaemonInstanceId, DaemonKind, Product, ServerIdentity};

pub use agentro_contracts::{DigestError, Sha256Digest};
pub use occurrence::{
    FencingToken, Lease, LeaseError, LeaseOwnerId, Occurrence, OccurrenceId, OccurrenceState,
    ScheduleRevision, TaskId, UtcInstant, select_misfires,
};
pub use policy::{
    CronDialect, CronExpression, DstFoldPolicy, DstGapPolicy, IanaTimeZone, MisfirePolicy,
    OverlapPolicy, PolicyError, RetryPolicy, SchedulePolicy,
};
pub use port::{
    AgentrodPort, CompileWorkflowPort, CompileWorkflowRequest, CompileWorkflowResponse,
    DispatchLookup, DispatchRequest, DispatchStart, OrchestrationRunId, PortError,
};

/// The unchanged Segno distribution identity.
pub const PRODUCT: Product = Product::SegnoFlow;
/// The daemon that owns schedules, occurrences, and dispatch intent.
pub const DAEMON: DaemonKind = DaemonKind::Segnod;

/// Creates a type-safe Segno daemon identity for bootstrap metadata.
#[must_use]
pub const fn server_identity(instance_id: DaemonInstanceId) -> ServerIdentity {
    ServerIdentity::new(DAEMON, instance_id)
}

#[cfg(test)]
mod tests {
    use agentro_contracts::DaemonInstanceId;

    use super::{PRODUCT, server_identity};

    #[test]
    fn preserves_segno_distribution_identity() {
        let identity = server_identity(DaemonInstanceId::generate());

        assert_eq!(PRODUCT.distribution_name(), "segno-flow");
        assert_eq!(identity.product(), PRODUCT);
        assert_eq!(identity.daemon().executable_name(), "segnod");
    }
}
