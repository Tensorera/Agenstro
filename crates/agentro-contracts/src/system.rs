use thiserror::Error;

use crate::{CapabilitySet, DaemonInstanceId, Sha256Digest};

const MIN_PROTOBUF_TIMESTAMP_SECONDS: i64 = -62_135_596_800;
const MAX_PROTOBUF_TIMESTAMP_SECONDS: i64 = 253_402_300_799;
const NANOS_PER_SECOND: u32 = 1_000_000_000;
const MAX_RELEASE_VERSION_BYTES: usize = 64;
const MAX_TARGET_TRIPLE_BYTES: usize = 128;
const MAX_BUILD_COMMIT_BYTES: usize = 128;

/// One compatible API major and minor version.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ApiVersion {
    major: u32,
    minor: u32,
}

impl ApiVersion {
    /// Creates an API version with a non-zero major.
    ///
    /// # Errors
    ///
    /// Returns [`SystemContractError::InvalidApiMajor`] when `major` is zero.
    pub const fn new(major: u32, minor: u32) -> Result<Self, SystemContractError> {
        if major == 0 {
            return Err(SystemContractError::InvalidApiMajor);
        }
        Ok(Self { major, minor })
    }

    /// Returns the API major.
    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    /// Returns the additive API minor.
    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }
}

/// A compatible minor range within exactly one API major.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ApiVersionRange {
    minimum: ApiVersion,
    maximum: ApiVersion,
}

impl ApiVersionRange {
    /// Creates an inclusive API compatibility range.
    ///
    /// # Errors
    ///
    /// Returns [`SystemContractError`] when the versions use different majors
    /// or the maximum minor precedes the minimum minor.
    pub const fn new(
        minimum: ApiVersion,
        maximum: ApiVersion,
    ) -> Result<Self, SystemContractError> {
        if minimum.major != maximum.major {
            return Err(SystemContractError::MixedApiMajors);
        }
        if minimum.minor > maximum.minor {
            return Err(SystemContractError::ReversedApiRange);
        }
        Ok(Self { minimum, maximum })
    }

    /// Returns the oldest compatible API version.
    #[must_use]
    pub const fn minimum(self) -> ApiVersion {
        self.minimum
    }

    /// Returns the newest compatible API version.
    #[must_use]
    pub const fn maximum(self) -> ApiVersion {
        self.maximum
    }

    /// Returns whether one concrete API version is inside this range.
    #[must_use]
    pub const fn supports(self, version: ApiVersion) -> bool {
        version.major == self.minimum.major
            && version.minor >= self.minimum.minor
            && version.minor <= self.maximum.minor
    }

    /// Selects the newest minor accepted by both ranges.
    #[must_use]
    pub const fn negotiate(self, peer: Self) -> Option<ApiVersion> {
        if self.minimum.major != peer.minimum.major {
            return None;
        }
        let minimum_minor = if self.minimum.minor > peer.minimum.minor {
            self.minimum.minor
        } else {
            peer.minimum.minor
        };
        let maximum_minor = if self.maximum.minor < peer.maximum.minor {
            self.maximum.minor
        } else {
            peer.maximum.minor
        };
        if minimum_minor > maximum_minor {
            None
        } else {
            Some(ApiVersion {
                major: self.minimum.major,
                minor: maximum_minor,
            })
        }
    }
}

/// A UTC timestamp compatible with `google.protobuf.Timestamp`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UtcTimestamp {
    seconds: i64,
    nanos: u32,
}

impl UtcTimestamp {
    /// Creates a validated UTC timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`SystemContractError::InvalidTimestamp`] when the seconds are
    /// outside years 0001 through 9999 or nanoseconds reach one second.
    pub const fn new(seconds: i64, nanos: u32) -> Result<Self, SystemContractError> {
        if seconds < MIN_PROTOBUF_TIMESTAMP_SECONDS
            || seconds > MAX_PROTOBUF_TIMESTAMP_SECONDS
            || nanos >= NANOS_PER_SECOND
        {
            return Err(SystemContractError::InvalidTimestamp);
        }
        Ok(Self { seconds, nanos })
    }

    /// Returns seconds since the Unix epoch.
    #[must_use]
    pub const fn seconds(self) -> i64 {
        self.seconds
    }

    /// Returns the non-negative nanosecond fraction.
    #[must_use]
    pub const fn nanos(self) -> u32 {
        self.nanos
    }
}

/// An unchanged public product or distribution identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Product {
    /// The `clef-sdk` distribution and directory.
    ClefSdk,
    /// The `tactus-runtime` distribution and directory.
    TactusRuntime,
    /// The `motivo-studio` distribution and directory.
    MotivoStudio,
    /// The `segno-flow` distribution and directory.
    SegnoFlow,
}

impl Product {
    /// Returns the unchanged external distribution name.
    #[must_use]
    pub const fn distribution_name(self) -> &'static str {
        match self {
            Self::ClefSdk => "clef-sdk",
            Self::TactusRuntime => "tactus-runtime",
            Self::MotivoStudio => "motivo-studio",
            Self::SegnoFlow => "segno-flow",
        }
    }
}

/// One independent daemon process and failure domain.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DaemonKind {
    /// Owns Clef workflow and normalized agent orchestration.
    Agentrod,
    /// Owns execution, workspace transactions, and Python workers.
    Tactusd,
    /// Owns schedules, occurrences, leases, and dispatch intent.
    Segnod,
}

impl DaemonKind {
    /// Returns the internal daemon executable name.
    #[must_use]
    pub const fn executable_name(self) -> &'static str {
        match self {
            Self::Agentrod => "agentrod",
            Self::Tactusd => "tactusd",
            Self::Segnod => "segnod",
        }
    }

    /// Returns the unchanged public product served by this daemon.
    #[must_use]
    pub const fn product(self) -> Product {
        match self {
            Self::Agentrod => Product::ClefSdk,
            Self::Tactusd => Product::TactusRuntime,
            Self::Segnod => Product::SegnoFlow,
        }
    }
}

/// Product, daemon, and per-launch instance identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerIdentity {
    daemon: DaemonKind,
    instance_id: DaemonInstanceId,
}

impl ServerIdentity {
    /// Creates a server identity; the public product is derived from the daemon.
    #[must_use]
    pub const fn new(daemon: DaemonKind, instance_id: DaemonInstanceId) -> Self {
        Self {
            daemon,
            instance_id,
        }
    }

    /// Returns the unchanged public product identity.
    #[must_use]
    pub const fn product(self) -> Product {
        self.daemon.product()
    }

    /// Returns the daemon failure domain.
    #[must_use]
    pub const fn daemon(self) -> DaemonKind {
        self.daemon
    }

    /// Returns the per-launch instance identifier.
    #[must_use]
    pub const fn instance_id(self) -> DaemonInstanceId {
        self.instance_id
    }
}

/// Validated, bounded build provenance shown by `GetServerInfo`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildInfo {
    release_version: Box<str>,
    target_triple: Box<str>,
    build_commit: Option<Box<str>>,
}

impl BuildInfo {
    /// Creates bounded build provenance text.
    ///
    /// # Errors
    ///
    /// Returns [`SystemContractError::InvalidBuildText`] for empty,
    /// non-ASCII-graphic, or oversized values.
    pub fn new(
        release_version: &str,
        target_triple: &str,
        build_commit: Option<&str>,
    ) -> Result<Self, SystemContractError> {
        validate_build_text(
            "release_version",
            release_version,
            MAX_RELEASE_VERSION_BYTES,
        )?;
        validate_build_text("target_triple", target_triple, MAX_TARGET_TRIPLE_BYTES)?;
        if let Some(commit) = build_commit {
            validate_build_text("build_commit", commit, MAX_BUILD_COMMIT_BYTES)?;
        }

        Ok(Self {
            release_version: release_version.into(),
            target_triple: target_triple.into(),
            build_commit: build_commit.map(Into::into),
        })
    }

    /// Returns the product release version.
    #[must_use]
    pub fn release_version(&self) -> &str {
        &self.release_version
    }

    /// Returns the compiled Rust target triple.
    #[must_use]
    pub fn target_triple(&self) -> &str {
        &self.target_triple
    }

    /// Returns the optional source commit identifier.
    #[must_use]
    pub fn build_commit(&self) -> Option<&str> {
        self.build_commit.as_deref()
    }
}

fn validate_build_text(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), SystemContractError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(SystemContractError::InvalidBuildText { field });
    }
    Ok(())
}

/// Version, descriptor fingerprint, and bounded negotiated capabilities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolInfo {
    api_versions: ApiVersionRange,
    descriptor_digest: Sha256Digest,
    capabilities: CapabilitySet,
}

impl ProtocolInfo {
    /// Creates protocol compatibility metadata from already validated values.
    #[must_use]
    pub const fn new(
        api_versions: ApiVersionRange,
        descriptor_digest: Sha256Digest,
        capabilities: CapabilitySet,
    ) -> Self {
        Self {
            api_versions,
            descriptor_digest,
            capabilities,
        }
    }

    /// Returns the inclusive API compatibility range.
    #[must_use]
    pub const fn api_versions(&self) -> ApiVersionRange {
        self.api_versions
    }

    /// Returns the protocol descriptor fingerprint.
    #[must_use]
    pub const fn descriptor_digest(&self) -> Sha256Digest {
        self.descriptor_digest
    }

    /// Returns capabilities in canonical name order.
    #[must_use]
    pub const fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }
}

/// Transport-independent daemon metadata behind `GetServerInfo`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerInfo {
    identity: ServerIdentity,
    build: BuildInfo,
    protocol: ProtocolInfo,
    started_at: UtcTimestamp,
}

impl ServerInfo {
    /// Creates complete server metadata from validated components.
    #[must_use]
    pub const fn new(
        identity: ServerIdentity,
        build: BuildInfo,
        protocol: ProtocolInfo,
        started_at: UtcTimestamp,
    ) -> Self {
        Self {
            identity,
            build,
            protocol,
            started_at,
        }
    }

    /// Returns product, daemon, and instance identity.
    #[must_use]
    pub const fn identity(&self) -> ServerIdentity {
        self.identity
    }

    /// Returns bounded build provenance.
    #[must_use]
    pub const fn build(&self) -> &BuildInfo {
        &self.build
    }

    /// Returns protocol compatibility metadata.
    #[must_use]
    pub const fn protocol(&self) -> &ProtocolInfo {
        &self.protocol
    }

    /// Returns the daemon start instant.
    #[must_use]
    pub const fn started_at(&self) -> UtcTimestamp {
        self.started_at
    }
}

/// Externally visible subset of daemon lifecycle state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HealthState {
    /// Startup is incomplete and work admission is closed.
    Starting,
    /// The daemon may accept authorized work.
    Serving,
    /// New long-running work is rejected while owned work drains.
    Draining,
}

/// Point-in-time health DTO independent of gRPC generated types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthReport {
    state: HealthState,
    checked_at: UtcTimestamp,
}

impl HealthReport {
    /// Creates a health report from explicit lifecycle state and time.
    #[must_use]
    pub const fn new(state: HealthState, checked_at: UtcTimestamp) -> Self {
        Self { state, checked_at }
    }

    /// Returns the externally visible lifecycle state.
    #[must_use]
    pub const fn state(self) -> HealthState {
        self.state
    }

    /// Returns the instant at which health was sampled.
    #[must_use]
    pub const fn checked_at(self) -> UtcTimestamp {
        self.checked_at
    }
}

/// A daemon system contract invariant violation.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SystemContractError {
    /// API major zero is reserved for unspecified wire values.
    #[error("API major must be non-zero")]
    InvalidApiMajor,
    /// A compatibility range cannot cross API majors.
    #[error("API compatibility range uses different majors")]
    MixedApiMajors,
    /// The maximum minor precedes the minimum minor.
    #[error("API compatibility range is reversed")]
    ReversedApiRange,
    /// The UTC timestamp is outside the Protobuf range or has invalid nanos.
    #[error("UTC timestamp is outside the Protobuf range")]
    InvalidTimestamp,
    /// A bounded build field is empty, oversized, or not ASCII graphic text.
    #[error("build field {field} is invalid")]
    InvalidBuildText {
        /// The stable field name that failed validation.
        field: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use crate::{CapabilitySet, DaemonInstanceId, Sha256Digest};

    use super::{
        ApiVersion, ApiVersionRange, BuildInfo, DaemonKind, Product, ProtocolInfo, ServerIdentity,
        ServerInfo, SystemContractError, UtcTimestamp,
    };

    #[test]
    fn api_range_stays_within_one_major() -> Result<(), SystemContractError> {
        let minimum = ApiVersion::new(1, 0)?;
        let maximum = ApiVersion::new(1, 4)?;
        let range = ApiVersionRange::new(minimum, maximum)?;

        assert_eq!(range.minimum(), minimum);
        assert_eq!(range.maximum(), maximum);
        assert!(range.supports(ApiVersion::new(1, 2)?));
        assert_eq!(
            range.negotiate(ApiVersionRange::new(
                ApiVersion::new(1, 2)?,
                ApiVersion::new(1, 6)?,
            )?),
            Some(ApiVersion::new(1, 4)?)
        );
        assert_eq!(
            range.negotiate(ApiVersionRange::new(
                ApiVersion::new(1, 5)?,
                ApiVersion::new(1, 6)?,
            )?),
            None
        );
        assert_eq!(
            ApiVersionRange::new(minimum, ApiVersion::new(2, 0)?),
            Err(SystemContractError::MixedApiMajors)
        );
        Ok(())
    }

    #[test]
    fn server_identity_cannot_mismatch_public_product() {
        let identity = ServerIdentity::new(DaemonKind::Agentrod, DaemonInstanceId::generate());

        assert_eq!(identity.product(), Product::ClefSdk);
        assert_eq!(identity.product().distribution_name(), "clef-sdk");
        assert_eq!(identity.daemon().executable_name(), "agentrod");
    }

    #[test]
    fn server_info_uses_domain_values_not_wire_types() -> Result<(), SystemContractError> {
        let identity = ServerIdentity::new(DaemonKind::Tactusd, DaemonInstanceId::generate());
        let build = BuildInfo::new("0.1.0", "x86_64-pc-windows-msvc", Some("abc123"))?;
        let protocol = ProtocolInfo::new(
            ApiVersionRange::new(ApiVersion::new(1, 0)?, ApiVersion::new(1, 0)?)?,
            Sha256Digest::from_bytes([7; 32]),
            CapabilitySet::default(),
        );
        let started_at = UtcTimestamp::new(1_787_000_000, 0)?;
        let info = ServerInfo::new(identity, build, protocol, started_at);

        assert_eq!(info.identity().product(), Product::TactusRuntime);
        assert_eq!(info.build().release_version(), "0.1.0");
        assert_eq!(info.started_at(), started_at);
        Ok(())
    }
}
