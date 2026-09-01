//! Canonical outcome classification and safe reconciliation diagnostics.

use serde::{Deserialize, Serialize};

use crate::{
    process::{InvocationKind, InvocationPhase, ProcessOutcome},
    protocol::TerminalResult,
};

/// Stable user/operator classification independent from transport internals.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeState {
    /// A valid success result and successful process exit were observed.
    Succeeded,
    /// A definite, authoritative failure was observed.
    Failed,
    /// External work may have happened without an authoritative terminal fact.
    OutcomeUnknown,
}

impl OutcomeState {
    /// Stable lowercase name accepted by `tactus runs --state` filters.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }
}

/// Classify a complete process outcome, including a provider host that
/// reported `error.code = outcome_unknown` in a valid terminal failure frame.
#[must_use]
pub fn classify_outcome(outcome: &ProcessOutcome) -> OutcomeState {
    if outcome_is_unknown(outcome) {
        OutcomeState::OutcomeUnknown
    } else if outcome.kind == InvocationKind::Succeeded {
        OutcomeState::Succeeded
    } else {
        OutcomeState::Failed
    }
}

/// Whether retry could duplicate an external effect.
#[must_use]
pub fn outcome_is_unknown(outcome: &ProcessOutcome) -> bool {
    matches!(
        outcome.kind,
        InvocationKind::ProcessFailed
            | InvocationKind::ProtocolFailed
            | InvocationKind::RuntimeFailed
            | InvocationKind::DeadlineExceeded
            | InvocationKind::Cancelled
    ) || matches!(
        outcome.terminal.as_ref(),
        Some(TerminalResult::Failure { error }) if error.code == "outcome_unknown"
    )
}

/// Validate the combinations that give the normalized invocation kind its
/// meaning.  This is intentionally shared by journal writers and readers so a
/// hand-written or corrupted summary cannot be mistaken for trustworthy
/// terminal evidence.
pub fn validate_outcome_consistency(outcome: &ProcessOutcome) -> Result<(), &'static str> {
    match outcome.kind {
        InvocationKind::Succeeded => {
            if outcome.exit_code != Some(0) {
                return Err("a succeeded invocation must have exit code zero");
            }
            if !matches!(outcome.terminal, Some(TerminalResult::Success { .. })) {
                return Err("a succeeded invocation must contain a success terminal result");
            }
        }
        InvocationKind::PluginFailed => {
            if !matches!(outcome.terminal, Some(TerminalResult::Failure { .. })) {
                return Err("a plugin failure must contain a failure terminal result");
            }
        }
        InvocationKind::ProcessFailed => {
            if !matches!(outcome.terminal, Some(TerminalResult::Success { .. })) {
                return Err("a process failure must contain the contradicted success result");
            }
            if outcome.exit_code == Some(0) {
                return Err("a process failure cannot have exit code zero");
            }
        }
        InvocationKind::ProtocolFailed
        | InvocationKind::RuntimeFailed
        | InvocationKind::DeadlineExceeded
        | InvocationKind::Cancelled => {}
    }
    Ok(())
}

/// Safe correlation metadata supplied by Tactus or Segno.  The business key
/// itself is intentionally absent; only its SHA-256 may cross this boundary.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutcomeContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_key_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Versioned, allowlisted diagnostic for one ambiguous outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OutcomeUnknownDiagnostic {
    pub api: &'static str,
    #[serde(flatten)]
    pub context: OutcomeContext,
    pub namespace: String,
    pub method: String,
    pub phase: &'static str,
    pub dispatched: bool,
    pub first_response_received: bool,
    pub partial_output_generated: bool,
    pub terminal_received: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_unix_ms: Option<u64>,
    pub frames_seen: u64,
    pub events_dropped: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_response_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_unix_ms: Option<u64>,
    pub external_effect_possible: bool,
    pub cleanup_completed: bool,
    pub reconciliation: Vec<&'static str>,
}

impl OutcomeUnknownDiagnostic {
    /// Build a diagnostic exclusively from typed context and supervisor facts.
    #[must_use]
    pub fn from_outcome(
        context: OutcomeContext,
        namespace: impl Into<String>,
        method: impl Into<String>,
        outcome: &ProcessOutcome,
    ) -> Self {
        let progress = outcome.progress.as_ref();
        let phase = progress.map_or("before_spawn", |progress| match progress.phase {
            InvocationPhase::Dispatched => "dispatched",
            InvocationPhase::FirstResponse => "first_response",
            InvocationPhase::PartialOutput => "partial_output",
            InvocationPhase::TerminalReceived => "terminal_received",
        });
        let external_effect_possible = outcome_is_unknown(outcome) && progress.is_some();
        let dispatched = progress.is_some();
        let first_response_received =
            progress.is_some_and(|value| value.first_response_unix_ms.is_some());
        let terminal_received =
            progress.is_some_and(|value| value.phase == InvocationPhase::TerminalReceived);
        let partial_output_generated = progress.is_some_and(|value| {
            value.phase == InvocationPhase::PartialOutput
                || (value.phase == InvocationPhase::TerminalReceived && outcome.frames_seen > 1)
        });
        let reconciliation = if external_effect_possible {
            vec![
                "Inspect the named external provider or system using the occurrence and run identifiers.",
                "Inspect workspace outputs without mutating them.",
                "Do not retry until the external result has been reconciled.",
            ]
        } else {
            vec![
                "Correct the local configuration or executable resolution failure.",
                "Retry only after confirming that no external process was dispatched.",
            ]
        };
        Self {
            api: "agenstro.outcome-unknown/v1",
            context,
            namespace: namespace.into(),
            method: method.into(),
            phase,
            dispatched,
            first_response_received,
            partial_output_generated,
            terminal_received,
            dispatched_unix_ms: progress.map(|value| value.dispatched_unix_ms),
            frames_seen: outcome.frames_seen,
            events_dropped: outcome.events_dropped,
            first_response_unix_ms: progress.and_then(|value| value.first_response_unix_ms),
            last_event_unix_ms: progress.and_then(|value| value.last_event_unix_ms),
            external_effect_possible,
            cleanup_completed: progress.is_some_and(|value| value.cleanup_completed),
            reconciliation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process::{InvocationProgress, ProcessOutcome},
        protocol::PluginFailure,
    };

    fn failed_with(code: &str) -> ProcessOutcome {
        ProcessOutcome {
            kind: InvocationKind::PluginFailed,
            exit_code: Some(1),
            terminal: Some(TerminalResult::Failure {
                error: PluginFailure {
                    code: code.to_owned(),
                    message: "bounded fixture".to_owned(),
                    details: None,
                },
            }),
            frames_seen: 1,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms: 5,
            progress: Some(InvocationProgress {
                phase: InvocationPhase::TerminalReceived,
                dispatched_unix_ms: 10,
                first_response_unix_ms: Some(12),
                last_event_unix_ms: Some(12),
                cleanup_completed: true,
            }),
        }
    }

    #[test]
    fn structured_provider_unknown_is_not_flattened_to_plugin_failed() {
        assert_eq!(
            classify_outcome(&failed_with("outcome_unknown")),
            OutcomeState::OutcomeUnknown
        );
        assert_eq!(
            classify_outcome(&failed_with("validation_failed")),
            OutcomeState::Failed
        );
    }

    #[test]
    fn outcome_consistency_rejects_contradictory_terminal_facts() {
        let mut value = failed_with("validation_failed");
        value.kind = InvocationKind::Succeeded;
        assert!(validate_outcome_consistency(&value).is_err());

        value.terminal = Some(TerminalResult::Success {
            value: serde_json::json!({"ok": true}),
        });
        assert!(validate_outcome_consistency(&value).is_err());
        value.exit_code = Some(0);
        assert!(validate_outcome_consistency(&value).is_ok());
    }

    #[test]
    fn diagnostic_contains_only_typed_progress_and_safe_context() {
        let diagnostic = OutcomeUnknownDiagnostic::from_outcome(
            OutcomeContext {
                workflow: Some("010_work.hs".to_owned()),
                task: Some("daily-task".to_owned()),
                business_key_sha256: Some("ab".repeat(32)),
                occurrence_id: Some("occ:fixture".to_owned()),
                provider: Some("claude-code".to_owned()),
            },
            "provider",
            "invoke",
            &failed_with("outcome_unknown"),
        );
        assert_eq!(diagnostic.phase, "terminal_received");
        assert!(diagnostic.external_effect_possible);
        assert!(diagnostic.dispatched);
        assert!(diagnostic.first_response_received);
        assert!(diagnostic.terminal_received);
        assert!(!diagnostic.partial_output_generated);
        assert_eq!(diagnostic.dispatched_unix_ms, Some(10));
        assert_eq!(diagnostic.last_event_unix_ms, Some(12));
        let encoded = serde_json::to_string(&diagnostic).expect("diagnostic JSON");
        assert!(!encoded.contains("prompt"));
        assert!(!encoded.contains("business_key\""));
    }
}
