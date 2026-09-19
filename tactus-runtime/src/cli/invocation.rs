//! Shared execution and journal lifecycle for CLI and plugin-v1 invocations.

use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, TryLockError},
    time::Duration,
};

use serde_json::json;

use super::{
    CliError, InvocationReport, ToolRuntime, apply_transport_limits,
    attach_builtin_provider_to_supervised_group, create_journal_preserving_execution,
    diagnostic_frame, finish_journal_preserving_outcome, in_memory_degraded_summary,
    note_journal_degradation, outcome_unknown_diagnostic_value, presentation_for_frame,
    process_error_outcome, record_outcome_transition, resolve_builtin_command,
    safe_invocation_subject, safe_presentation_identifier,
};
use crate::{
    journal::{
        JournalError, Presentation, PresentationCategory, RunJournal, TransitionGuard,
        TransitionTrigger, TriggerKind,
    },
    limits::RuntimeLimits,
    outcome::outcome_is_unknown,
    process::{CancellationToken, ProcessSpec, ProcessSupervisor},
    protocol::{PluginFrame, PluginRequest},
    workspace::{ResolvedPlugin, Workspace},
};

/// The entry point changes diagnostic provenance, not supervision policy.
#[derive(Clone, Copy)]
pub(super) enum InvocationOrigin {
    Cli,
    Dispatch,
}

pub(super) struct InvocationProcess<'a> {
    pub workspace: &'a Workspace,
    pub definition: ResolvedPlugin<'a>,
    pub limits: &'a RuntimeLimits,
    pub timeout_seconds: u64,
    pub cancellation: &'a CancellationToken,
}

pub(super) struct InvocationCompletion {
    pub report: InvocationReport,
    pub journal_degradation: Option<String>,
}

/// Own one journal from the requested transition through terminal publication.
pub(super) struct Invocation {
    name: String,
    registry: String,
    method: String,
    origin: InvocationOrigin,
    journal: RunJournal,
    journal_error: Option<JournalError>,
    run_path: PathBuf,
    run_id: String,
    pub started_presentation: Presentation,
}

impl Invocation {
    pub fn start(
        workspace: &Workspace,
        name: &str,
        registry: &str,
        method: &str,
        command: &[String],
        origin: InvocationOrigin,
    ) -> Self {
        let (mut journal, mut journal_error) = create_journal_preserving_execution(workspace);
        let run_path = journal.run_path().to_path_buf();
        let run_id = journal.run_id().to_owned();
        let started_presentation = Presentation::new(
            PresentationCategory::State,
            format!(
                "{} started.",
                safe_invocation_subject(registry, name, method)
            ),
        );
        let (trigger, guard) = match origin {
            InvocationOrigin::Cli => {
                let executable = command
                    .first()
                    .and_then(|value| Path::new(value).file_name())
                    .map_or_else(|| "<unknown>".into(), |value| value.to_string_lossy());
                (
                    TransitionTrigger::new(
                        TriggerKind::Request,
                        "tactus.cli",
                        "plugin.invocation_requested",
                    )
                    .with_details(json!({
                        "plugin":name,"method":method,"namespace":registry,
                        "executable":executable,
                        "argument_count":command.len().saturating_sub(1),
                    })),
                    TransitionGuard::new(
                        "plugin resolved and request validated",
                        true,
                        "The configured plugin and invocation request passed runtime validation.",
                    ),
                )
            }
            InvocationOrigin::Dispatch => (
                TransitionTrigger::new(
                    TriggerKind::Request,
                    "tactus.dispatch",
                    "plugin.dispatch_requested",
                )
                .with_details(json!({
                    "plugin":name,"namespace":registry,"method":method,
                    "parent_run_id":env::var("TACTUS_RUN_ID").ok(),
                })),
                TransitionGuard::new(
                    "dispatch request and plugin registry entry validated",
                    true,
                    "The request belongs to a configured plugin and passed protocol validation.",
                ),
            ),
        };
        if journal_error.is_none() {
            journal_error = journal
                .record_transition(
                    "ready",
                    trigger,
                    guard,
                    "running",
                    started_presentation.clone(),
                )
                .err();
        }
        Self {
            name: name.to_owned(),
            registry: registry.to_owned(),
            method: method.to_owned(),
            origin,
            journal,
            journal_error,
            run_path,
            run_id,
            started_presentation,
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Record every frame before handing it to the entry point's output sink.
    /// A stalled or failed journal must not replace the known process outcome.
    pub fn run(
        self,
        process: InvocationProcess<'_>,
        request: &PluginRequest,
        mut sink: impl FnMut(&PluginFrame) + Send + 'static,
    ) -> Result<InvocationCompletion, CliError> {
        let tool_runtime = ToolRuntime::create(process.workspace)?;
        let mut spec = ProcessSpec::new(
            resolve_builtin_command(process.definition.command())?,
            &process.workspace.root,
        );
        apply_transport_limits(&mut spec, process.limits);
        spec.environment = tool_runtime.environment.clone();
        spec.environment
            .insert("TACTUS_WATCH_RUN_ID".to_owned(), self.run_id.clone());
        attach_builtin_provider_to_supervised_group(&mut spec, process.definition.command());
        spec.limits.deadline =
            (process.timeout_seconds != 0).then(|| Duration::from_secs(process.timeout_seconds));
        let mut shared_state = SharedJournal::new(self.journal);
        shared_state.error = self.journal_error;
        let shared_journal = Arc::new(Mutex::new(shared_state));
        let callback_journal = Arc::clone(&shared_journal);
        let display_name = self.name.clone();
        let result = ProcessSupervisor.invoke(&spec, request, process.cancellation, move |frame| {
            callback_journal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .record_frame(&display_name, frame);
            sink(frame);
        });
        let outcome = result.unwrap_or_else(|error| process_error_outcome(&error));
        let (mut journal, mut degradation) = match take_shared_journal(&shared_journal) {
            Ok((journal, error)) => (Some(journal), error.map(|error| error.to_string())),
            Err(error) => (None, Some(error.to_string())),
        };
        if let Some(journal) = journal.as_mut()
            && degradation.is_none()
        {
            if let Err(error) = record_outcome_transition(
                journal,
                &self.registry,
                &self.name,
                &self.method,
                &outcome,
            ) {
                note_journal_degradation(&mut degradation, error);
            }
            if degradation.is_none() && outcome_is_unknown(&outcome) {
                let subject = match self.origin {
                    InvocationOrigin::Cli => format!("{}:{}", self.registry, self.name),
                    InvocationOrigin::Dispatch => safe_presentation_identifier(&self.name),
                };
                if let Err(error) = journal.record_with_presentation(
                    "runtime.outcome_unknown",
                    outcome_unknown_diagnostic_value(
                        &self.registry, &self.name, &self.method, &outcome,
                    ),
                    Some(Presentation::new(
                        PresentationCategory::Warning,
                        format!("{subject} may have completed externally; Tactus did not retry it automatically."),
                    )),
                ) {
                    note_journal_degradation(&mut degradation, error);
                }
            }
        }
        let was_durable = journal.as_ref().is_some_and(RunJournal::is_durable);
        let (summary, journal_degradation) = match journal.as_mut() {
            Some(journal) => finish_journal_preserving_outcome(journal, outcome, degradation),
            None => {
                let diagnostic =
                    degradation.unwrap_or_else(|| "the journal writer was unavailable".to_owned());
                (
                    in_memory_degraded_summary(&self.run_id, outcome, &diagnostic),
                    Some(diagnostic),
                )
            }
        };
        Ok(InvocationCompletion {
            report: InvocationReport {
                name: self.name,
                run_path: self.run_path,
                summary,
                persisted: was_durable && journal_degradation.is_none(),
            },
            journal_degradation,
        })
    }
}

struct SharedJournal {
    journal: Option<RunJournal>,
    error: Option<JournalError>,
}

impl SharedJournal {
    fn new(journal: RunJournal) -> Self {
        Self {
            journal: Some(journal),
            error: None,
        }
    }

    fn record_frame(&mut self, subject: &str, frame: &PluginFrame) {
        if self.error.is_some() {
            return;
        }
        let Some(journal) = self.journal.as_mut() else {
            return;
        };
        let (kind, data) = diagnostic_frame(frame);
        if let Err(error) =
            journal.record_with_presentation(kind, data, presentation_for_frame(subject, frame))
        {
            self.error = Some(error);
        }
    }
}

fn take_shared_journal(
    shared: &Arc<Mutex<SharedJournal>>,
) -> Result<(RunJournal, Option<JournalError>), CliError> {
    let mut state = match shared.try_lock() {
        Ok(state) => state,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            return Err(CliError::EventSinkStalled(
                "run journal writer did not return before supervision ended".to_owned(),
            ));
        }
    };
    let journal = state
        .journal
        .take()
        .ok_or_else(|| CliError::EventSinkStalled("run journal was already taken".to_owned()))?;
    Ok((journal, state.error.take()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        journal::RunSummary,
        process::{InvocationKind, ProcessOutcome},
        protocol::{PluginFailure, TerminalResult},
    };
    use std::{fs, io};
    use tempfile::tempdir;

    #[test]
    fn journal_callback_failure_preserves_known_terminal_outcomes() {
        let outcomes = [
            ProcessOutcome {
                kind: InvocationKind::Succeeded,
                exit_code: Some(0),
                terminal: Some(TerminalResult::Success {
                    value: serde_json::json!({"result":"known success"}),
                }),
                frames_seen: 1,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: None,
                elapsed_ms: 1,
                progress: None,
            },
            ProcessOutcome {
                kind: InvocationKind::PluginFailed,
                exit_code: Some(1),
                terminal: Some(TerminalResult::Failure {
                    error: PluginFailure {
                        code: "domain_failure".to_owned(),
                        message: "known failure".to_owned(),
                        details: Some(serde_json::json!({"stage":"fixture"})),
                    },
                }),
                frames_seen: 1,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: None,
                elapsed_ms: 1,
                progress: None,
            },
        ];

        for expected in outcomes {
            let temporary = tempdir().expect("temporary directory");
            let workspace = Workspace::at(temporary.path());
            let journal = RunJournal::create(&workspace).expect("journal");
            let mut shared_state = SharedJournal::new(journal);
            shared_state.error = Some(JournalError::Io(io::Error::other(
                "injected callback writer failure",
            )));
            let shared = Arc::new(Mutex::new(shared_state));
            let (mut journal, callback_error) = take_shared_journal(&shared).expect("take journal");
            let (summary, degradation) = finish_journal_preserving_outcome(
                &mut journal,
                expected.clone(),
                callback_error.map(|error| error.to_string()),
            );

            assert_eq!(summary.outcome.kind, expected.kind);
            assert_eq!(summary.outcome.terminal, expected.terminal);
            assert!(
                summary
                    .outcome
                    .observation_error
                    .as_deref()
                    .is_some_and(|message| message.contains("injected callback writer failure"))
            );
            assert!(
                degradation
                    .as_deref()
                    .is_some_and(|message| message.contains("injected callback writer failure"))
            );
            let durable: RunSummary = serde_json::from_slice(
                &fs::read(journal.summary_path()).expect("degraded summary"),
            )
            .expect("summary JSON");
            assert_eq!(durable.outcome.kind, expected.kind);
            assert_eq!(
                durable
                    .outcome
                    .terminal
                    .as_ref()
                    .map(|terminal| match terminal {
                        TerminalResult::Success { .. } => "success",
                        TerminalResult::Failure { .. } => "failure",
                    }),
                expected.terminal.as_ref().map(|terminal| match terminal {
                    TerminalResult::Success { .. } => "success",
                    TerminalResult::Failure { .. } => "failure",
                })
            );
        }
    }
}
