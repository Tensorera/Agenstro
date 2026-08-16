use std::{fmt, str::FromStr};

use agentro_contracts::{
    Capability, CapabilityError, CapabilityName, CapabilitySet, CapabilityStability,
};
use clef_core::Effort;

use crate::AgentEvent;

const MAX_ID_BYTES: usize = 128;
const MAX_ADAPTER_VERSION_BYTES: usize = 64;
const MAX_CAPABILITY_REQUESTS: usize = 64;
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

/// A provider-neutral agent contract construction error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentContractError {
    /// A bounded identifier is empty, malformed, or oversized.
    InvalidIdentifier,
    /// A bounded text field is empty or oversized.
    InvalidText,
    /// A collection exceeds its hard item limit.
    LimitExceeded,
    /// A capability is duplicated or both required and preferred.
    InvalidCapabilityRequest,
    /// An event limit is zero or exceeds the protocol hard limit.
    InvalidEventLimit,
}

impl fmt::Display for AgentContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => "agent identifier is invalid",
            Self::InvalidText => "agent text field is invalid",
            Self::LimitExceeded => "agent collection exceeds its hard limit",
            Self::InvalidCapabilityRequest => "agent capability request is invalid",
            Self::InvalidEventLimit => "agent event limit is invalid",
        })
    }
}

impl std::error::Error for AgentContractError {}

fn validate_id(value: &str) -> Result<(), AgentContractError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AgentContractError::InvalidIdentifier);
    }
    Ok(())
}

macro_rules! define_agent_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Box<str>);

        impl $name {
            /// Parses the bounded canonical identifier.
            ///
            /// # Errors
            ///
            /// Returns [`AgentContractError::InvalidIdentifier`] for malformed input.
            pub fn parse(value: &str) -> Result<Self, AgentContractError> {
                validate_id(value)?;
                Ok(Self(value.into()))
            }

            /// Returns canonical identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = AgentContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

define_agent_id!(
    /// Core-owned session identity, distinct from any provider session ID.
    AgentSessionId
);
define_agent_id!(
    /// Core-owned turn identity within one session.
    AgentTurnId
);
define_agent_id!(
    /// Normalized adapter/provider installation name.
    ProviderName
);
define_agent_id!(
    /// Opaque authorized workspace identity, never an unrestricted host path.
    WorkspaceId
);

/// Major/minor normalized adapter protocol version.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProtocolVersion {
    major: u16,
    minor: u16,
}

impl ProtocolVersion {
    /// Current normalized adapter protocol version.
    pub const V1: Self = Self { major: 1, minor: 0 };

    /// Creates a version with a non-zero major.
    ///
    /// # Errors
    ///
    /// Returns [`AgentContractError::InvalidIdentifier`] when `major` is zero.
    pub const fn new(major: u16, minor: u16) -> Result<Self, AgentContractError> {
        if major == 0 {
            return Err(AgentContractError::InvalidIdentifier);
        }
        Ok(Self { major, minor })
    }

    /// Returns the breaking major.
    #[must_use]
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Returns the additive minor.
    #[must_use]
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

/// Stable normalized coding-agent capabilities.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AgentCapability {
    /// Incremental content/event streaming.
    Streaming,
    /// Persistent sessions.
    Sessions,
    /// Resume a compatible prior session.
    Resume,
    /// Provider permission approvals.
    Approvals,
    /// Structured model output.
    StructuredOutput,
    /// Provider-reported file-change events.
    FileChangeEvents,
    /// Token or cost usage reports.
    Usage,
    /// Explicit model selection.
    ModelSelection,
    /// Logical effort routing.
    ReasoningEffort,
    /// MCP integration.
    Mcp,
    /// Provider-native subagents.
    Subagents,
    /// Supervised terminal tool access.
    Terminal,
    /// Native context compaction.
    Compact,
    /// Turn or cost budget enforcement.
    Budget,
}

impl AgentCapability {
    /// Returns the stable namespaced capability name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Streaming => "agent.streaming",
            Self::Sessions => "agent.sessions",
            Self::Resume => "agent.resume",
            Self::Approvals => "agent.approvals",
            Self::StructuredOutput => "agent.structured-output",
            Self::FileChangeEvents => "agent.file-change-events",
            Self::Usage => "agent.usage",
            Self::ModelSelection => "agent.model-selection",
            Self::ReasoningEffort => "agent.reasoning-effort",
            Self::Mcp => "agent.mcp",
            Self::Subagents => "agent.subagents",
            Self::Terminal => "agent.terminal",
            Self::Compact => "agent.compact",
            Self::Budget => "agent.budget",
        }
    }

    /// Builds a common stable capability value.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError`] only if a built-in name violates the shared
    /// contract, which indicates an implementation defect.
    pub fn capability(self) -> Result<Capability, CapabilityError> {
        Ok(Capability::new(
            CapabilityName::parse(self.as_str())?,
            CapabilityStability::Stable,
        ))
    }
}

/// Required and preferred capabilities for one session/turn.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilityRequest {
    required: Vec<CapabilityName>,
    preferred: Vec<CapabilityName>,
}

impl CapabilityRequest {
    /// Creates a deterministic bounded capability request.
    ///
    /// # Errors
    ///
    /// Rejects duplicates, required/preferred overlap, and more than 64 names
    /// in either set.
    pub fn new(
        mut required: Vec<CapabilityName>,
        mut preferred: Vec<CapabilityName>,
    ) -> Result<Self, AgentContractError> {
        if required.len() > MAX_CAPABILITY_REQUESTS || preferred.len() > MAX_CAPABILITY_REQUESTS {
            return Err(AgentContractError::LimitExceeded);
        }
        required.sort();
        preferred.sort();
        if required.windows(2).any(|pair| pair[0] == pair[1])
            || preferred.windows(2).any(|pair| pair[0] == pair[1])
            || required.iter().any(|item| preferred.contains(item))
        {
            return Err(AgentContractError::InvalidCapabilityRequest);
        }
        Ok(Self {
            required,
            preferred,
        })
    }

    /// Returns hard requirements in canonical name order.
    #[must_use]
    pub fn required(&self) -> &[CapabilityName] {
        &self.required
    }

    /// Returns preferred capabilities in canonical name order.
    #[must_use]
    pub fn preferred(&self) -> &[CapabilityName] {
        &self.preferred
    }

    /// Returns missing hard requirements in canonical order.
    #[must_use]
    pub fn missing_required(&self, available: &CapabilitySet) -> Vec<CapabilityName> {
        self.required
            .iter()
            .filter(|capability| !available.contains(capability))
            .cloned()
            .collect()
    }

    /// Returns explicit preferred-capability degradations in canonical order.
    #[must_use]
    pub fn missing_preferred(&self, available: &CapabilitySet) -> Vec<CapabilityName> {
        self.preferred
            .iter()
            .filter(|capability| !available.contains(capability))
            .cloned()
            .collect()
    }
}

/// Highest connectivity level requested from an adapter host.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProbeLevel {
    /// Resolve and execute a version check.
    Installed,
    /// Read provider-native authentication status without a prompt.
    Authentication,
    /// Initialize and shut down the machine protocol without a prompt.
    Protocol,
    /// Explicitly authorized live model turn.
    Live,
}

/// Prompt-free by default adapter probe request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeRequest {
    level: ProbeLevel,
}

impl ProbeRequest {
    /// Creates a probe request for the given maximum level.
    #[must_use]
    pub const fn new(level: ProbeLevel) -> Self {
        Self { level }
    }

    /// Returns the requested maximum level.
    #[must_use]
    pub const fn level(self) -> ProbeLevel {
        self.level
    }
}

/// Structured adapter handshake/probe result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeReport {
    provider: ProviderName,
    adapter_version: Box<str>,
    protocol: ProtocolVersion,
    capabilities: CapabilitySet,
    is_authenticated: bool,
}

impl ProbeReport {
    /// Creates a bounded probe report.
    ///
    /// # Errors
    ///
    /// Returns [`AgentContractError::InvalidText`] for an empty, non-graphic,
    /// or oversized adapter version.
    pub fn new(
        provider: ProviderName,
        adapter_version: &str,
        protocol: ProtocolVersion,
        capabilities: CapabilitySet,
        is_authenticated: bool,
    ) -> Result<Self, AgentContractError> {
        if adapter_version.is_empty()
            || adapter_version.len() > MAX_ADAPTER_VERSION_BYTES
            || !adapter_version.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(AgentContractError::InvalidText);
        }
        Ok(Self {
            provider,
            adapter_version: adapter_version.into(),
            protocol,
            capabilities,
            is_authenticated,
        })
    }

    /// Returns the normalized provider/installation name.
    #[must_use]
    pub const fn provider(&self) -> &ProviderName {
        &self.provider
    }

    /// Returns the bounded adapter-host version.
    #[must_use]
    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }

    /// Returns the normalized protocol version.
    #[must_use]
    pub const fn protocol(&self) -> ProtocolVersion {
        self.protocol
    }

    /// Returns dynamically negotiated capabilities.
    #[must_use]
    pub const fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Returns the read-only provider authentication status.
    #[must_use]
    pub const fn is_authenticated(&self) -> bool {
        self.is_authenticated
    }
}

/// Request to open one single-owner adapter session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenSessionRequest {
    session_id: AgentSessionId,
    workspace_id: WorkspaceId,
    capabilities: CapabilityRequest,
    effort: Option<Effort>,
    max_events: u32,
}

impl OpenSessionRequest {
    /// Creates a bounded session request.
    ///
    /// # Errors
    ///
    /// Returns [`AgentContractError::InvalidEventLimit`] unless `max_events` is
    /// in `1..=4096`.
    pub fn new(
        session_id: AgentSessionId,
        workspace_id: WorkspaceId,
        capabilities: CapabilityRequest,
        effort: Option<Effort>,
        max_events: u32,
    ) -> Result<Self, AgentContractError> {
        if !(1..=4_096).contains(&max_events) {
            return Err(AgentContractError::InvalidEventLimit);
        }
        Ok(Self {
            session_id,
            workspace_id,
            capabilities,
            effort,
            max_events,
        })
    }

    /// Returns the core-owned session ID.
    #[must_use]
    pub const fn session_id(&self) -> &AgentSessionId {
        &self.session_id
    }

    /// Returns the authorized opaque workspace identity.
    #[must_use]
    pub const fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    /// Returns required and preferred capabilities.
    #[must_use]
    pub const fn capabilities(&self) -> &CapabilityRequest {
        &self.capabilities
    }

    /// Returns the logical Clef effort, independent of provider variants.
    #[must_use]
    pub const fn effort(&self) -> Option<Effort> {
        self.effort
    }

    /// Returns the hard maximum events accepted for the session turn.
    #[must_use]
    pub const fn max_events(&self) -> u32 {
        self.max_events
    }
}

/// One bounded turn input sent to an already opened session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInput {
    turn_id: AgentTurnId,
    instruction: Box<str>,
}

impl AgentInput {
    /// Creates a bounded provider-neutral turn input.
    ///
    /// # Errors
    ///
    /// Returns [`AgentContractError::InvalidText`] for empty, NUL-containing,
    /// or larger-than-1-MiB instructions.
    pub fn new(turn_id: AgentTurnId, instruction: &str) -> Result<Self, AgentContractError> {
        if instruction.trim().is_empty()
            || instruction.len() > MAX_PROMPT_BYTES
            || instruction.contains('\0')
        {
            return Err(AgentContractError::InvalidText);
        }
        Ok(Self {
            turn_id,
            instruction: instruction.into(),
        })
    }

    /// Returns the core-owned turn ID.
    #[must_use]
    pub const fn turn_id(&self) -> &AgentTurnId {
        &self.turn_id
    }

    /// Returns the provider-neutral instruction.
    #[must_use]
    pub fn instruction(&self) -> &str {
        &self.instruction
    }
}

/// Stable normalized backend error code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AgentErrorCode {
    /// Adapter installation is unavailable.
    NotInstalled,
    /// Provider authentication is unavailable or expired.
    AuthenticationMissing,
    /// Adapter protocol is incompatible or malformed.
    ProtocolViolation,
    /// A hard requested capability was not negotiated.
    CapabilityMissing,
    /// Requested model route is unavailable.
    ModelUnavailable,
    /// Policy denied the requested operation.
    PermissionDenied,
    /// Deadline expired.
    Timeout,
    /// Cancellation completed.
    Cancelled,
    /// Bounded output/event capacity was exhausted.
    OutputLimit,
    /// Adapter host or provider process died.
    ProcessDied,
    /// Bounded internal adapter failure.
    Internal,
}

/// Provider-neutral backend error without raw provider payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentError {
    code: AgentErrorCode,
}

impl AgentError {
    /// Creates a normalized error from a stable code.
    #[must_use]
    pub const fn new(code: AgentErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable machine code.
    #[must_use]
    pub const fn code(self) -> AgentErrorCode {
        self.code
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "normalized agent error: {:?}", self.code)
    }
}

impl std::error::Error for AgentError {}

/// Explicit reason propagated through adapter-native cancellation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CancelReason {
    /// User or API request.
    User,
    /// Overall application deadline.
    Deadline,
    /// Output/event hard limit.
    OutputLimit,
    /// Owning daemon shutdown.
    Shutdown,
}

/// Normalized cancellation outcome.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CancelResult {
    /// Adapter/provider acknowledged native cancellation.
    Acknowledged,
    /// Supervisor had to force termination.
    Forced,
    /// The provider turn was already terminal.
    AlreadyTerminal,
}

/// Adapter-host port implemented outside the Clef domain/application crates.
pub trait AgentBackend: Send + Sync {
    /// Performs the requested connectivity probe without exceeding its level.
    fn probe(&self, request: ProbeRequest) -> Result<ProbeReport, AgentError>;

    /// Opens one session after capability negotiation.
    fn open_session(
        &self,
        request: OpenSessionRequest,
    ) -> Result<Box<dyn AgentSession>, AgentError>;
}

/// Single-owner normalized session. Implementations must bound every call.
pub trait AgentSession: Send {
    /// Sends exactly one turn input; repeated sends require explicit adapter support.
    fn send(&mut self, input: AgentInput) -> Result<(), AgentError>;

    /// Polls at most one event without creating an unbounded internal queue.
    fn poll_event(&mut self) -> Result<Option<AgentEvent>, AgentError>;

    /// Propagates cancellation through provider-native and supervisor stages.
    fn cancel(&mut self, reason: CancelReason) -> Result<CancelResult, AgentError>;

    /// Performs bounded explicit close. Consuming `self` enforces one close owner.
    fn close(self: Box<Self>) -> Result<(), AgentError>;
}
