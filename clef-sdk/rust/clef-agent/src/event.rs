use std::{collections::BTreeSet, fmt};

use agentro_contracts::UtcTimestamp;
use clef_core::{ProducedArtifact, ProjectPath};

use crate::{
    AgentContractError, AgentErrorCode, AgentSessionId, AgentTurnId, ProtocolVersion, ProviderName,
};

/// Maximum normalized payload bytes accepted for one agent event.
pub const MAX_AGENT_EVENT_BYTES: u32 = 1024 * 1024;
/// Maximum normalized events accepted for one turn.
pub const MAX_AGENT_EVENTS_PER_TURN: u32 = 4_096;
const MAX_CONTENT_DELTA_BYTES: usize = 64 * 1_024;
const MAX_EVENT_LABEL_BYTES: usize = 256;
const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1_024;
const MAX_TURN_ARTIFACTS: usize = 64;

/// Portable physical file-change category reported by an adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FileChangeKind {
    /// A path was created.
    Created,
    /// Existing path content was modified.
    Modified,
    /// A path was deleted.
    Deleted,
}

/// Stable normalized diagnostic category.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AgentDiagnosticCode {
    /// A safe-to-ignore provider notification had no normalized mapping.
    UnknownProviderEvent,
    /// The adapter explicitly degraded a preferred capability.
    CapabilityDegraded,
    /// Bounded non-terminal adapter warning.
    AdapterWarning,
}

/// Provider-attributed usage counters without provider-private fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageReport {
    input_tokens: u64,
    output_tokens: u64,
    is_final: bool,
}

impl UsageReport {
    /// Creates normalized usage counters.
    #[must_use]
    pub const fn new(input_tokens: u64, output_tokens: u64, is_final: bool) -> Self {
        Self {
            input_tokens,
            output_tokens,
            is_final,
        }
    }

    /// Returns provider-attributed input tokens.
    #[must_use]
    pub const fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    /// Returns provider-attributed output tokens.
    #[must_use]
    pub const fn output_tokens(&self) -> u64 {
        self.output_tokens
    }

    /// Returns whether this is the provider's final usage report.
    #[must_use]
    pub const fn is_final(&self) -> bool {
        self.is_final
    }
}

/// Stable intersection of adapter-host events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEventBody {
    /// The normalized provider session started.
    SessionStarted,
    /// Incremental UTF-8 content.
    ContentDelta {
        /// Bounded content fragment.
        text: Box<str>,
    },
    /// A normalized tool invocation started.
    ToolStarted {
        /// Bounded normalized tool name.
        name: Box<str>,
    },
    /// A normalized tool invocation completed.
    ToolCompleted {
        /// Bounded normalized tool name.
        name: Box<str>,
        /// Whether the tool completed successfully.
        succeeded: bool,
    },
    /// Provider permission requires a Clef policy decision.
    ApprovalRequested {
        /// Core/adapter-owned approval correlation ID.
        request_id: Box<str>,
    },
    /// A prior approval request was resolved.
    ApprovalResolved {
        /// Core/adapter-owned approval correlation ID.
        request_id: Box<str>,
        /// Whether policy approved the operation.
        approved: bool,
    },
    /// Provider reported a project-relative file change.
    FileChangeReported {
        /// Canonical project-relative path.
        path: ProjectPath,
        /// Reported physical change category.
        change: FileChangeKind,
    },
    /// Provider-attributed usage changed.
    UsageUpdated(UsageReport),
    /// Bounded normalized diagnostic.
    Diagnostic {
        /// Stable diagnostic category.
        code: AgentDiagnosticCode,
        /// Bounded redacted summary, never a raw provider payload.
        message: Box<str>,
    },
    /// Unique successful terminal event for the turn.
    TurnCompleted {
        /// Normalized produced artifacts in canonical slot order.
        artifacts: Vec<ProducedArtifact>,
    },
    /// Unique failed terminal event for the turn.
    TurnFailed {
        /// Stable provider-neutral failure category.
        code: AgentErrorCode,
    },
}

impl AgentEventBody {
    /// Returns whether this body terminates its turn.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::TurnCompleted { .. } | Self::TurnFailed { .. })
    }

    fn validate_and_size(&self) -> Result<u32, AgentContractError> {
        let size = match self {
            Self::SessionStarted | Self::TurnFailed { .. } => 1,
            Self::ContentDelta { text } => {
                validate_text(text, MAX_CONTENT_DELTA_BYTES)?;
                text.len()
            }
            Self::ToolStarted { name } | Self::ToolCompleted { name, .. } => {
                validate_text(name, MAX_EVENT_LABEL_BYTES)?;
                name.len()
            }
            Self::ApprovalRequested { request_id } | Self::ApprovalResolved { request_id, .. } => {
                validate_text(request_id, MAX_EVENT_LABEL_BYTES)?;
                request_id.len()
            }
            Self::FileChangeReported { path, .. } => path.as_str().len(),
            Self::UsageUpdated(_) => 17,
            Self::Diagnostic { message, .. } => {
                validate_text(message, MAX_DIAGNOSTIC_BYTES)?;
                message.len()
            }
            Self::TurnCompleted { artifacts } => {
                if artifacts.len() > MAX_TURN_ARTIFACTS {
                    return Err(AgentContractError::LimitExceeded);
                }
                let mut names = BTreeSet::new();
                let mut bytes = 0_usize;
                for artifact in artifacts {
                    if !names.insert(artifact.name().clone()) {
                        return Err(AgentContractError::InvalidIdentifier);
                    }
                    bytes = bytes.saturating_add(artifact.name().as_str().len() + 1);
                }
                bytes
            }
        };
        let size = u32::try_from(size).map_err(|_| AgentContractError::LimitExceeded)?;
        if size > MAX_AGENT_EVENT_BYTES {
            return Err(AgentContractError::LimitExceeded);
        }
        Ok(size)
    }
}

fn validate_text(value: &str, maximum: usize) -> Result<(), AgentContractError> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(AgentContractError::InvalidText);
    }
    Ok(())
}

/// One versioned, sequenced, bounded normalized agent event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentEvent {
    protocol: ProtocolVersion,
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    sequence: u64,
    occurred_at: UtcTimestamp,
    provider: ProviderName,
    payload_bytes: u32,
    body: AgentEventBody,
}

impl AgentEvent {
    /// Creates an event and computes its normalized payload size.
    ///
    /// # Errors
    ///
    /// Rejects sequence zero, oversized text/artifact sets, duplicate output
    /// names, or any payload above [`MAX_AGENT_EVENT_BYTES`].
    pub fn new(
        protocol: ProtocolVersion,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        sequence: u64,
        occurred_at: UtcTimestamp,
        provider: ProviderName,
        body: AgentEventBody,
    ) -> Result<Self, AgentContractError> {
        if sequence == 0 {
            return Err(AgentContractError::InvalidEventLimit);
        }
        let payload_bytes = body.validate_and_size()?;
        Ok(Self {
            protocol,
            session_id,
            turn_id,
            sequence,
            occurred_at,
            provider,
            payload_bytes,
            body,
        })
    }

    /// Returns the normalized adapter protocol version.
    #[must_use]
    pub const fn protocol(&self) -> ProtocolVersion {
        self.protocol
    }

    /// Returns the core-owned session ID.
    #[must_use]
    pub const fn session_id(&self) -> &AgentSessionId {
        &self.session_id
    }

    /// Returns the core-owned turn ID.
    #[must_use]
    pub const fn turn_id(&self) -> &AgentTurnId {
        &self.turn_id
    }

    /// Returns the one-based monotonically increasing sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the normalized UTC occurrence time.
    #[must_use]
    pub const fn occurred_at(&self) -> UtcTimestamp {
        self.occurred_at
    }

    /// Returns the normalized provider/installation name.
    #[must_use]
    pub const fn provider(&self) -> &ProviderName {
        &self.provider
    }

    /// Returns the computed normalized payload bytes.
    #[must_use]
    pub const fn payload_bytes(&self) -> u32 {
        self.payload_bytes
    }

    /// Returns the stable typed event body.
    #[must_use]
    pub const fn body(&self) -> &AgentEventBody {
        &self.body
    }
}

/// Stateful protocol validation failure for one event stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProtocolError {
    /// Adapter protocol major is unsupported.
    UnsupportedProtocol,
    /// Event belongs to another session, turn, or provider.
    CorrelationMismatch,
    /// Event sequence is not exactly the next expected value.
    SequenceViolation,
    /// Payload exceeds the configured per-event bound.
    PayloadLimit,
    /// Event count exceeds the configured per-turn bound.
    EventLimit,
    /// Any event appeared after the unique terminal event.
    EventAfterTerminal,
}

impl fmt::Display for AgentProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedProtocol => "agent event protocol major is unsupported",
            Self::CorrelationMismatch => "agent event correlation does not match the turn",
            Self::SequenceViolation => "agent event sequence is not monotonic and contiguous",
            Self::PayloadLimit => "agent event payload exceeds its configured limit",
            Self::EventLimit => "agent event count exceeds its configured limit",
            Self::EventAfterTerminal => "agent event appeared after the terminal event",
        })
    }
}

impl std::error::Error for AgentProtocolError {}

/// Single-owner validator for event correlation, ordering, limits, and terminality.
#[derive(Clone, Debug)]
pub struct AgentEventValidator {
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    provider: ProviderName,
    next_sequence: u64,
    accepted_events: u32,
    max_payload_bytes: u32,
    max_events: u32,
    is_terminal: bool,
}

impl AgentEventValidator {
    /// Creates a validator with explicit hard event and payload limits.
    ///
    /// # Errors
    ///
    /// Rejects zero or protocol-exceeding limits.
    pub fn new(
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        provider: ProviderName,
        max_payload_bytes: u32,
        max_events: u32,
    ) -> Result<Self, AgentContractError> {
        if !(1..=MAX_AGENT_EVENT_BYTES).contains(&max_payload_bytes)
            || !(1..=MAX_AGENT_EVENTS_PER_TURN).contains(&max_events)
        {
            return Err(AgentContractError::InvalidEventLimit);
        }
        Ok(Self {
            session_id,
            turn_id,
            provider,
            next_sequence: 1,
            accepted_events: 0,
            max_payload_bytes,
            max_events,
            is_terminal: false,
        })
    }

    /// Validates exactly one event and advances state only on success.
    ///
    /// # Errors
    ///
    /// Returns [`AgentProtocolError`] for protocol/correlation mismatch,
    /// non-contiguous sequence, bound exhaustion, or post-terminal output.
    pub fn accept(&mut self, event: &AgentEvent) -> Result<(), AgentProtocolError> {
        if self.is_terminal {
            return Err(AgentProtocolError::EventAfterTerminal);
        }
        if event.protocol().major() != ProtocolVersion::V1.major() {
            return Err(AgentProtocolError::UnsupportedProtocol);
        }
        if event.session_id() != &self.session_id
            || event.turn_id() != &self.turn_id
            || event.provider() != &self.provider
        {
            return Err(AgentProtocolError::CorrelationMismatch);
        }
        if event.sequence() != self.next_sequence {
            return Err(AgentProtocolError::SequenceViolation);
        }
        if event.payload_bytes() > self.max_payload_bytes {
            return Err(AgentProtocolError::PayloadLimit);
        }
        if self.accepted_events >= self.max_events {
            return Err(AgentProtocolError::EventLimit);
        }

        self.accepted_events += 1;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.is_terminal = event.body().is_terminal();
        Ok(())
    }

    /// Returns whether a unique terminal event was accepted.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.is_terminal
    }

    /// Returns the next required sequence.
    #[must_use]
    pub const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
}
