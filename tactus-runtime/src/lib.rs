//! Tactus is the typed, project-local execution kernel around Clef workflows.
//!
//! Clef owns workflow semantics. Tactus owns workspace discovery, process
//! supervision, streaming plugin transport, and factual run journals.

pub mod adapters;
pub mod cli;
pub mod journal;
pub mod process;
pub mod protocol;
pub mod studio;
pub mod workspace;

pub use journal::{RunJournal, RunSummary, TraceEvent};
pub use process::{
    CancellationToken, CommandKind, CommandOutcome, InvocationKind, ProcessLimits, ProcessOutcome,
    ProcessSpec, ProcessSupervisor,
};
pub use protocol::{
    FrameSequence, JsonField, PluginEvent, PluginFailure, PluginFrame, PluginRequest,
    ProtocolFault, RequestId, TerminalResult, decode_request,
};
pub use workspace::{
    EffectDefinition, PluginDefinition, ProviderDefinition, ResolvedPlugin, RuntimeConfig,
    ScriptInfo, Workspace,
};
