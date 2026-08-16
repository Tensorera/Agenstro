//! Provider-neutral Clef agent port and fake adapter-host conformance surface.
//!
//! No Codex, Claude, OpenCode, ACP, provider CLI, or provider SDK type appears
//! in this crate. Adapter hosts translate their private wire protocols into
//! these bounded capabilities, requests, events, and errors.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod contract;
mod event;
mod fake;

pub use contract::{
    AgentBackend, AgentCapability, AgentContractError, AgentError, AgentErrorCode, AgentInput,
    AgentSession, AgentSessionId, AgentTurnId, CancelReason, CancelResult, CapabilityRequest,
    OpenSessionRequest, ProbeLevel, ProbeReport, ProbeRequest, ProtocolVersion, ProviderName,
    WorkspaceId,
};
pub use event::{
    AgentDiagnosticCode, AgentEvent, AgentEventBody, AgentEventValidator, AgentProtocolError,
    FileChangeKind, MAX_AGENT_EVENT_BYTES, MAX_AGENT_EVENTS_PER_TURN, UsageReport,
};
pub use fake::{FakeBackend, FakeBackendMetrics};
