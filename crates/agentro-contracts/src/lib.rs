//! Pure, transport-independent contracts shared by the Rust control plane.
//!
//! This crate owns validated value objects only. It deliberately has no Tokio,
//! storage, filesystem, gRPC, operating-system, or provider SDK dependency.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod capability;
mod digest;
mod error;
mod id;
mod pagination;
mod system;

pub use capability::{
    Capability, CapabilityError, CapabilityName, CapabilitySet, CapabilityStability,
    MAX_CAPABILITIES, MAX_CAPABILITY_NAME_BYTES,
};
pub use digest::{
    CanonicalHasher, DigestError, MAX_CANONICAL_FIELD_BYTES, MAX_CANONICAL_FIELDS, Sha256Digest,
};
pub use error::{
    ErrorCode, ErrorCodeError, ErrorDescriptor, ErrorDomain, MAX_ERROR_CODE_BYTES, RetryAdvice,
};
pub use id::{DaemonInstanceId, IdError, RequestId, TraceId};
pub use pagination::{DEFAULT_PAGE_ITEMS, MAX_PAGE_ITEMS, PageBudget, PageBudgetError};
pub use system::{
    ApiVersion, ApiVersionRange, BuildInfo, DaemonKind, HealthReport, HealthState, Product,
    ProtocolInfo, ServerIdentity, ServerInfo, SystemContractError, UtcTimestamp,
};
