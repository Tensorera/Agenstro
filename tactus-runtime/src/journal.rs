//! Append-only run events and atomically published summaries.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{outcome::validate_outcome_consistency, process::ProcessOutcome, workspace::Workspace};

/// Version of the factual trace envelope. It does not promise replay.
pub const TRACE_API: &str = "agenstro.trace/v1";

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

/// One sequenced event in `events.jsonl`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TraceEvent {
    /// Trace schema version.
    pub api: String,
    /// Directory/run correlation identifier.
    pub run_id: String,
    /// One-based order assigned by this journal.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub at_unix_ms: u64,
    /// Runtime-defined event category.
    pub kind: String,
    /// Optional human projection. Structured clients display this text instead
    /// of guessing a message from the diagnostic payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    /// Structured factual payload.
    pub data: Value,
}

/// Human-facing projection attached to a persisted diagnostic event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Presentation {
    /// One of the four public log categories.
    pub category: PresentationCategory,
    /// Natural-language text with no embedded structured payload.
    pub message: String,
}

impl Presentation {
    /// Construct a bounded presentation. Newlines are flattened so one event
    /// always occupies one human log line.
    #[must_use]
    pub fn new(category: PresentationCategory, message: impl Into<String>) -> Self {
        // Presentation strings may include plugin-owned error text.  Flatten
        // every control character so neither terminals nor Studio logs can be
        // manipulated with escape sequences.
        let message = message
            .into()
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>();
        Self {
            category,
            message: truncate_utf8(&message, 1_024),
        }
    }
}

/// Stable categories allowed in Shell and Studio user logs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationCategory {
    /// A real lifecycle state changed.
    State,
    /// Non-terminal progress or useful context.
    Info,
    /// Execution continued or stopped conservatively after a degraded condition.
    Warning,
    /// A known failure occurred.
    Error,
}

impl PresentationCategory {
    /// Text prefix used by terminal projections.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Source category for a state-transition trigger.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// A user, workflow, or control-plane request.
    Request,
    /// An externally observed event.
    Event,
    /// A logical or monotonic timer.
    Timer,
    /// A completed internal operation.
    InternalResult,
    /// Cancellation, shutdown, or another control signal.
    Control,
}

/// What requested one state transition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TransitionTrigger {
    /// Trigger family.
    pub kind: TriggerKind,
    /// Stable component that observed the trigger.
    pub source: String,
    /// Stable machine-readable trigger code.
    pub code: String,
    /// Optional bounded evidence; it is redacted before persistence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl TransitionTrigger {
    /// Create a trigger without optional evidence.
    #[must_use]
    pub fn new(kind: TriggerKind, source: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            kind,
            source: source.into(),
            code: code.into(),
            details: None,
        }
    }

    /// Attach structured evidence.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Why a requested state transition was permitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionGuard {
    /// Stable condition that was evaluated.
    pub condition: String,
    /// Whether the condition allowed this transition.
    pub passed: bool,
    /// Human-readable diagnostic reason for the decision.
    pub reason: String,
}

impl TransitionGuard {
    /// Construct one guard result.
    #[must_use]
    pub fn new(condition: impl Into<String>, passed: bool, reason: impl Into<String>) -> Self {
        Self {
            condition: condition.into(),
            passed,
            reason: reason.into(),
        }
    }
}

/// Required diagnostic shape for every real runtime state change.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StateTransition {
    /// State before the trigger was applied.
    pub state_before: String,
    /// What requested the transition.
    pub trigger: TransitionTrigger,
    /// Why the transition was allowed.
    pub guard: TransitionGuard,
    /// State after the transition was committed.
    pub state_after: String,
}

/// Atomically published terminal record.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunSummary {
    /// Trace schema version.
    pub api: String,
    /// Directory/run correlation identifier.
    pub run_id: String,
    /// Journal creation time.
    pub started_unix_ms: u64,
    /// Summary publication time.
    pub finished_unix_ms: u64,
    /// Number of recorded JSONL events.
    pub events_recorded: u64,
    /// Complete structured process result.
    pub outcome: ProcessOutcome,
}

/// Open writer for one run directory.
pub struct RunJournal {
    run_id: String,
    run_path: PathBuf,
    event_path: PathBuf,
    summary_path: PathBuf,
    started_unix_ms: u64,
    next_seq: u64,
    events: Option<BufWriter<File>>,
    finished: bool,
}

impl RunJournal {
    /// Create a unique run directory below `.tactus/runs`.
    pub fn create(workspace: &Workspace) -> Result<Self, JournalError> {
        ensure_safe_journal_root(workspace)?;
        let started_unix_ms = unix_millis();
        let (run_id, run_path) = loop {
            let ordinal = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
            let run_id = format!("run-{started_unix_ms}-{}-{ordinal}", std::process::id());
            let run_path = workspace.runs_path.join(&run_id);
            match fs::create_dir(&run_path) {
                Ok(()) => break (run_id, run_path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(JournalError::Io(error)),
            }
        };
        let event_path = run_path.join("events.jsonl");
        let events = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&event_path)
            .map(BufWriter::new)
            .map_err(|error| {
                // A directory without its mandatory event stream is not a
                // journal. Best-effort cleanup prevents a transient create
                // failure from becoming a durable false-corrupt record.
                let _ = fs::remove_dir(&run_path);
                JournalError::Io(error)
            })?;
        let summary_path = run_path.join("summary.json");
        Ok(Self {
            run_id,
            run_path,
            event_path,
            summary_path,
            started_unix_ms,
            next_seq: 1,
            events: Some(events),
            finished: false,
        })
    }

    /// Create an in-memory correlation record when durable storage is
    /// unavailable. Calls remain valid and produce an honest summary, but no
    /// event or summary file is claimed to exist.
    #[must_use]
    pub fn degraded(workspace: &Workspace) -> Self {
        let started_unix_ms = unix_millis();
        let ordinal = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let run_id = format!(
            "run-unpersisted-{started_unix_ms}-{}-{ordinal}",
            std::process::id()
        );
        let run_path = workspace.runs_path.join(&run_id);
        Self {
            event_path: run_path.join("events.jsonl"),
            summary_path: run_path.join("summary.json"),
            run_id,
            run_path,
            started_unix_ms,
            next_seq: 1,
            events: None,
            finished: false,
        }
    }

    /// Whether this journal owns durable files below `.tactus/runs`.
    #[must_use]
    pub const fn is_durable(&self) -> bool {
        self.events.is_some()
    }

    /// Stable run identifier.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Directory containing this run's records.
    #[must_use]
    pub fn run_path(&self) -> &Path {
        &self.run_path
    }

    /// Append and flush one event so a live reader can observe it.
    pub fn record(&mut self, kind: impl Into<String>, data: Value) -> Result<u64, JournalError> {
        self.record_with_presentation(kind, data, None)
    }

    /// Append one diagnostic event with an optional human projection.
    pub fn record_with_presentation(
        &mut self,
        kind: impl Into<String>,
        data: Value,
        presentation: Option<Presentation>,
    ) -> Result<u64, JournalError> {
        if self.finished {
            return Err(JournalError::AlreadyFinished);
        }
        let seq = self.next_seq;
        let event = TraceEvent {
            api: TRACE_API.to_owned(),
            run_id: self.run_id.clone(),
            seq,
            at_unix_ms: unix_millis(),
            kind: kind.into(),
            presentation,
            data: redact_diagnostic_value(data),
        };
        if let Some(events) = self.events.as_mut() {
            serde_json::to_writer(&mut *events, &event).map_err(JournalError::Json)?;
            events.write_all(b"\n").map_err(JournalError::Io)?;
            events.flush().map_err(JournalError::Io)?;
        }
        self.next_seq += 1;
        Ok(seq)
    }

    /// Persist one real state change using the mandatory four-part contract.
    pub fn record_transition(
        &mut self,
        state_before: impl Into<String>,
        trigger: TransitionTrigger,
        guard: TransitionGuard,
        state_after: impl Into<String>,
        presentation: Presentation,
    ) -> Result<u64, JournalError> {
        let transition = StateTransition {
            state_before: state_before.into(),
            trigger,
            guard,
            state_after: state_after.into(),
        };
        self.record_with_presentation(
            "runtime.state_transition",
            serde_json::to_value(transition).map_err(JournalError::Json)?,
            Some(presentation),
        )
    }

    /// Flush events and atomically publish `summary.json`.
    pub fn finish(&mut self, outcome: ProcessOutcome) -> Result<RunSummary, JournalError> {
        if self.finished {
            return Err(JournalError::AlreadyFinished);
        }
        validate_outcome_consistency(&outcome).map_err(JournalError::InvalidOutcome)?;
        let summary = self.snapshot_summary(outcome);
        if let Some(events) = self.events.as_mut() {
            events.flush().map_err(JournalError::Io)?;
            events.get_ref().sync_all().map_err(JournalError::Io)?;
            self.publish_summary(&summary)?;
        }
        self.finished = true;
        Ok(summary)
    }

    /// Publish a terminal summary after the event stream has degraded.
    ///
    /// This deliberately skips flushing/syncing the failed event writer. The
    /// summary is still attempted independently so diagnostic loss cannot hide
    /// an already-known terminal result.
    pub fn finish_degraded(&mut self, outcome: ProcessOutcome) -> Result<RunSummary, JournalError> {
        if self.finished {
            return Err(JournalError::AlreadyFinished);
        }
        validate_outcome_consistency(&outcome).map_err(JournalError::InvalidOutcome)?;
        let summary = self.snapshot_summary(outcome);
        if self.events.is_some() {
            self.publish_summary(&summary)?;
        }
        self.finished = true;
        Ok(summary)
    }

    /// Construct the in-memory summary returned when durable publication also
    /// fails. This does not mutate the journal or claim persistence succeeded.
    #[must_use]
    pub fn snapshot_summary(&self, outcome: ProcessOutcome) -> RunSummary {
        RunSummary {
            api: TRACE_API.to_owned(),
            run_id: self.run_id.clone(),
            started_unix_ms: self.started_unix_ms,
            finished_unix_ms: unix_millis(),
            events_recorded: if self.is_durable() {
                self.next_seq - 1
            } else {
                0
            },
            outcome,
        }
    }

    fn publish_summary(&mut self, summary: &RunSummary) -> Result<(), JournalError> {
        let temporary = self.run_path.join(format!(
            ".summary-{}-{}.tmp",
            std::process::id(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(JournalError::Io)?;
        if let Err(error) = (|| {
            let persisted = diagnostic_summary(summary);
            serde_json::to_writer(&mut file, &persisted).map_err(JournalError::Json)?;
            file.write_all(b"\n").map_err(JournalError::Io)?;
            file.sync_all().map_err(JournalError::Io)
        })() {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        if let Err(error) = fs::rename(&temporary, &self.summary_path) {
            let _ = fs::remove_file(&temporary);
            return Err(JournalError::Io(error));
        }
        Ok(())
    }

    /// Path of the append-only event stream.
    #[must_use]
    pub fn event_path(&self) -> &Path {
        &self.event_path
    }

    /// Path of the terminal summary.
    #[must_use]
    pub fn summary_path(&self) -> &Path {
        &self.summary_path
    }
}

fn ensure_safe_journal_root(workspace: &Workspace) -> Result<(), JournalError> {
    let root = dunce::canonicalize(&workspace.root).map_err(JournalError::Io)?;
    ensure_plain_direct_child(&workspace.control, &root, ".tactus")?;
    let control = dunce::canonicalize(&workspace.control).map_err(JournalError::Io)?;
    ensure_plain_direct_child(&workspace.runs_path, &control, ".tactus/runs")
}

fn ensure_plain_direct_child(
    path: &Path,
    canonical_parent: &Path,
    label: &'static str,
) -> Result<(), JournalError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(JournalError::Io)?;
        }
        Err(error) => return Err(JournalError::Io(error)),
    }
    let metadata = fs::symlink_metadata(path).map_err(JournalError::Io)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(JournalError::UnsafePath(label));
    }
    let canonical = dunce::canonicalize(path).map_err(JournalError::Io)?;
    if canonical.parent() != Some(canonical_parent) {
        return Err(JournalError::UnsafePath(label));
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

pub(crate) fn diagnostic_summary(summary: &RunSummary) -> RunSummary {
    let mut summary = summary.clone();
    summary.outcome.stderr_truncated |= !summary.outcome.stderr.is_empty();
    summary.outcome.stderr.clear();
    if let Some(error) = summary.outcome.error.as_mut() {
        *error = diagnostic_text_summary(error);
    }
    if let Some(error) = summary.outcome.observation_error.as_mut() {
        *error = diagnostic_text_summary(error);
    }
    summary.outcome.terminal = summary
        .outcome
        .terminal
        .as_ref()
        .map(|terminal| match terminal {
            crate::protocol::TerminalResult::Success { value } => {
                crate::protocol::TerminalResult::Success {
                    value: diagnostic_value_summary(value),
                }
            }
            crate::protocol::TerminalResult::Failure { error } => {
                let mut error = error.clone();
                error.code = durable_diagnostic_code(&error.code);
                error.message = diagnostic_text_summary(&error.message);
                // Terminal details belong to the plugin and are not trusted to
                // claim runtime-owned reconciliation metadata. Typed runtime
                // diagnostics and validated Clef sidecars are projected at
                // their respective ingestion points instead.
                error.details = error.details.as_ref().map(diagnostic_value_summary);
                crate::protocol::TerminalResult::Failure { error }
            }
        });
    summary
}

/// Retain only explicitly safe reconciliation/validation fields from a
/// structured failure.  The complete payload is still represented by a size
/// and digest so an operator can correlate it with live output without
/// persisting prompts, paths, or provider text.
pub(crate) fn diagnostic_failure_details(value: &Value) -> Value {
    let summary = diagnostic_value_summary(value);
    let mut projected = project_failure_object(value, 0);
    projected.insert("withheld".to_owned(), summary);
    Value::Object(projected)
}

const MAX_FAILURE_PROJECTION_DEPTH: usize = 8;
const MAX_FAILURE_PROJECTION_ITEMS: usize = 128;

fn project_failure_object(value: &Value, depth: usize) -> serde_json::Map<String, Value> {
    let mut projected = serde_json::Map::new();
    if depth >= MAX_FAILURE_PROJECTION_DEPTH {
        return projected;
    }
    let Value::Object(object) = value else {
        return projected;
    };
    for (key, field) in object {
        if failure_field_is_allowlisted(key) {
            projected.insert(
                key.clone(),
                safe_failure_field(key, field, depth.saturating_add(1)),
            );
        }
    }
    projected
}

fn failure_field_is_allowlisted(key: &str) -> bool {
    matches!(
        key,
        "cause"
            | "code"
            | "details"
            | "error"
            | "exit_code"
            | "phase"
            | "progress"
            | "dispatched"
            | "first_response_received"
            | "partial_output_generated"
            | "terminal_received"
            | "dispatched_unix_ms"
            | "frames_seen"
            | "event_frames_seen"
            | "events_dropped"
            | "terminal_frame_seen"
            | "first_response_unix_ms"
            | "last_event_unix_ms"
            | "last_event"
            | "type"
            | "external_effect_possible"
            | "cleanup_completed"
            | "reconciliation"
            | "required"
            | "automatic_retry_safe"
            | "validation_failed"
            | "validator_stage"
            | "stage"
            | "rule"
            | "severity"
            | "expected"
            | "observed"
            | "provenance"
            | "max_bytes"
            | "max_line_bytes"
            | "max_stdout_bytes"
            | "max_result_bytes"
            | "result_bytes_at_least"
            | "timeout_seconds"
            | "valid_up_to"
    )
}

fn safe_failure_field(key: &str, value: &Value, depth: usize) -> Value {
    if depth >= MAX_FAILURE_PROJECTION_DEPTH {
        return diagnostic_value_summary(value);
    }
    if key == "expected" {
        return project_validation_expected(value);
    }
    if key == "observed" {
        return project_validation_observed(value);
    }
    if key == "provenance" {
        return project_validation_provenance(value);
    }
    if key == "reconciliation" {
        let Value::Object(fields) = value else {
            return diagnostic_value_summary(value);
        };
        let mut projected = serde_json::Map::new();
        for name in ["required", "automatic_retry_safe"] {
            if let Some(Value::Bool(flag)) = fields.get(name) {
                projected.insert(name.to_owned(), Value::Bool(*flag));
            }
        }
        projected.insert("withheld".to_owned(), diagnostic_value_summary(value));
        return Value::Object(projected);
    }
    if matches!(
        key,
        "code" | "phase" | "type" | "stage" | "rule" | "severity"
    ) {
        return value
            .as_str()
            .and_then(safe_validation_token)
            .map_or_else(|| diagnostic_value_summary(value), Value::String);
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text)
            if text.len() <= 512 && !text.chars().any(|character| character.is_control()) =>
        {
            Value::String(text.clone())
        }
        Value::Array(values) if values.len() <= MAX_FAILURE_PROJECTION_ITEMS => Value::Array(
            values
                .iter()
                .map(|item| match item {
                    Value::Object(_) => Value::Object(project_failure_object(item, depth)),
                    _ => safe_failure_field(key, item, depth.saturating_add(1)),
                })
                .collect(),
        ),
        Value::Object(_) if depth < MAX_FAILURE_PROJECTION_DEPTH => {
            Value::Object(project_failure_object(value, depth))
        }
        _ => diagnostic_value_summary(value),
    }
}

fn project_validation_expected(value: &Value) -> Value {
    let Value::Object(fields) = value else {
        return diagnostic_value_summary(value);
    };
    let mut projected = serde_json::Map::new();
    for key in ["statement", "guidance"] {
        if let Some(Value::String(text)) = fields.get(key) {
            projected.insert(key.to_owned(), Value::String(diagnostic_text_summary(text)));
        }
    }
    if let Some(spec) = fields.get("spec") {
        projected.insert("spec".to_owned(), project_validation_value(spec, 0));
    }
    projected.insert("withheld".to_owned(), diagnostic_value_summary(value));
    Value::Object(projected)
}

fn project_validation_observed(value: &Value) -> Value {
    let Value::Object(fields) = value else {
        return diagnostic_value_summary(value);
    };
    let mut projected = serde_json::Map::new();
    if let Some(Value::String(message)) = fields.get("message") {
        projected.insert(
            "message".to_owned(),
            Value::String(diagnostic_text_summary(message)),
        );
    }
    if let Some(locus) = fields.get("locus") {
        projected.insert("locus".to_owned(), project_validation_locus(locus));
    }
    if let Some(evidence) = fields.get("evidence") {
        projected.insert("evidence".to_owned(), project_validation_value(evidence, 0));
    }
    projected.insert("withheld".to_owned(), diagnostic_value_summary(value));
    Value::Object(projected)
}

fn project_validation_locus(value: &Value) -> Value {
    let Value::Object(fields) = value else {
        return diagnostic_value_summary(value);
    };
    let mut projected = serde_json::Map::new();
    for key in ["startLine", "startColumn", "endLine", "endColumn"] {
        if let Some(Value::Number(number)) = fields.get(key) {
            projected.insert(key.to_owned(), Value::Number(number.clone()));
        }
    }
    for key in ["artifact", "snippet"] {
        if let Some(field) = fields.get(key) {
            projected.insert(key.to_owned(), diagnostic_value_summary(field));
        }
    }
    projected.insert("withheld".to_owned(), diagnostic_value_summary(value));
    Value::Object(projected)
}

fn project_validation_provenance(value: &Value) -> Value {
    let Value::Object(fields) = value else {
        return diagnostic_value_summary(value);
    };
    let mut projected = serde_json::Map::new();
    if let Some(Value::String(kind)) = fields.get("kind") {
        projected.insert(
            "kind".to_owned(),
            safe_validation_token(kind)
                .map(Value::String)
                .unwrap_or_else(|| diagnostic_value_summary(&Value::String(kind.clone()))),
        );
    }
    for key in ["support", "total", "observations"] {
        if let Some(Value::Number(number)) = fields.get(key) {
            projected.insert(key.to_owned(), Value::Number(number.clone()));
        }
    }
    for key in ["author", "corpus"] {
        if let Some(field) = fields.get(key) {
            projected.insert(key.to_owned(), diagnostic_value_summary(field));
        }
    }
    projected.insert("withheld".to_owned(), diagnostic_value_summary(value));
    Value::Object(projected)
}

fn project_validation_value(value: &Value, depth: usize) -> Value {
    if depth >= 4 {
        return diagnostic_value_summary(value);
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => safe_validation_token(text)
            .map(Value::String)
            .unwrap_or_else(|| diagnostic_value_summary(value)),
        Value::Array(items) if items.len() <= MAX_FAILURE_PROJECTION_ITEMS => Value::Array(
            items
                .iter()
                .map(|item| project_validation_value(item, depth + 1))
                .collect(),
        ),
        // Open-domain objects can hide arbitrary provider or artefact text
        // behind innocent-looking keys. Preserve their identity, not content.
        Value::Object(_) | Value::Array(_) => diagnostic_value_summary(value),
    }
}

fn safe_validation_token(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'_' | b'-')))
    .then(|| value.to_owned())
}

/// Redact arbitrary plugin evidence before it reaches the durable diagnostic
/// stream. Provider text and credentials remain available to the live caller,
/// but persistence retains only their size and fingerprint.
#[must_use]
pub fn redact_diagnostic_value(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(&key)
                        || (key.eq_ignore_ascii_case("error") && value.is_string())
                        || (durable_identifier_key(&key)
                            && value
                                .as_str()
                                .is_some_and(|text| !public_durable_identifier(&key, text)))
                    {
                        redacted_value(&value)
                    } else {
                        redact_diagnostic_value(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .take(1_024)
                .map(redact_diagnostic_value)
                .collect(),
        ),
        Value::String(value) => Value::String(truncate_utf8(&value, 4_096)),
        scalar => scalar,
    }
}

fn durable_identifier_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().replace('-', "_").as_str(),
        "code" | "event_type" | "id" | "method" | "type"
    )
}

fn public_durable_identifier(key: &str, value: &str) -> bool {
    match key.to_ascii_lowercase().replace('-', "_").as_str() {
        "code" => public_diagnostic_code(value),
        "method" => matches!(
            value,
            "invoke"
                | "smoke"
                | "observe.begin"
                | "observe.end"
                | "snapshot"
                | "diff"
                | "commit"
                | "rollback"
                | "run"
        ),
        "type" => matches!(
            value,
            "event"
                | "result"
                | "state_transition"
                | "message"
                | "plugin_event"
                | "provider.progress"
                | "provider.tool.started"
                | "provider.tool.completed"
                | "workflow.progress"
                | "effect.progress"
                | "effect.warning"
        ),
        "event_type" | "id" => false,
        _ => false,
    }
}

fn public_diagnostic_code(value: &str) -> bool {
    matches!(
        value,
        "outcome_unknown"
            | "plugin.outcome_unknown"
            | "plugin.deadline_exceeded"
            | "plugin.transport_failed"
            | "plugin.protocol_failed"
            | "plugin.process_exit_failed"
            | "workflow.validation_failed"
            | "script_batch_failed"
            | "script_preparation_failed"
            | "script_supervisor_start_failed"
            | "provider_invocation_failed"
            | "provider_reported_failure"
            | "observer_cleanup_failed"
            | "generation_produced_no_script"
            | "workspace_inspection_failed"
            | "script_batch.requested"
            | "script_batch.build_succeeded"
            | "script.completed"
            | "script.outcome_unknown"
            | "script.failed"
            | "script_batch.completed"
            | "script_batch.outcome_unknown"
            | "script_batch.failed"
            | "workflow.generation_requested"
            | "workflow.generation_completed"
            | "workflow.requested"
            | "workflow.result.error"
            | "plugin.invocation_requested"
            | "plugin.dispatch_requested"
            | "plugin.cancellation_requested"
            | "plugin.supervision_completed"
            | "test.requested"
    ) || value
        .strip_prefix("invalid_identifier.")
        .is_some_and(|digest| {
            digest.len() == 16 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn durable_diagnostic_code(value: &str) -> String {
    if public_diagnostic_code(value) {
        value.to_owned()
    } else {
        format!(
            "invalid_identifier.{:.16}",
            format!("{:x}", Sha256::digest(value.as_bytes()))
        )
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    matches!(
        key.as_str(),
        "prompt"
            | "goal"
            | "text"
            | "content"
            | "raw"
            | "payload"
            | "stderr"
            | "native_stderr"
            | "message"
            | "diagnostic"
            | "exception"
            | "instructions"
            | "authorization"
            | "password"
            | "secret"
            | "api_key"
            | "access_token"
            | "refresh_token"
            | "credential"
            | "credentials"
            | "cookie"
            | "environment"
            | "extra_env"
            | "options"
            | "workspace"
            | "cwd"
            | "path"
            | "run_path"
    ) || key.ends_with("_secret")
        || key.ends_with("_password")
        || key.ends_with("_credential")
}

fn redacted_value(value: &Value) -> Value {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    json!({
        "redacted": true,
        "bytes": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
    })
}

pub(crate) fn diagnostic_value_summary(value: &Value) -> Value {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    let kind = match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    json!({
        "diagnostic_summary": true,
        "value_type": kind,
        "bytes": encoded.len(),
        "sha256": format!("{:x}", Sha256::digest(encoded)),
    })
}

pub(crate) fn diagnostic_text_summary(value: &str) -> String {
    format!(
        "diagnostic withheld ({} bytes; sha256 {:x})",
        value.len(),
        Sha256::digest(value.as_bytes())
    )
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Run journal persistence failure.
#[derive(Debug, Error)]
pub enum JournalError {
    /// Filesystem operation failed.
    #[error("run journal I/O failed: {0}")]
    Io(#[source] io::Error),
    /// JSON serialization failed.
    #[error("cannot encode run journal: {0}")]
    Json(#[source] serde_json::Error),
    /// A normalized kind contradicted its exit or terminal evidence.
    #[error("cannot publish an inconsistent run outcome: {0}")]
    InvalidOutcome(&'static str),
    /// A journal root was linked, reparsed, or escaped its expected parent.
    #[error("run journal path is unsafe: {0}")]
    UnsafePath(&'static str),
    /// A caller attempted to mutate a terminal journal.
    #[error("run journal is already finished")]
    AlreadyFinished,
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::process::InvocationKind;

    fn outcome() -> ProcessOutcome {
        ProcessOutcome {
            kind: InvocationKind::Succeeded,
            exit_code: Some(0),
            terminal: Some(crate::protocol::TerminalResult::Success {
                value: serde_json::json!({"fixture": true}),
            }),
            frames_seen: 1,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms: 1,
            progress: None,
        }
    }

    #[test]
    fn writes_ordered_events_then_atomic_summary() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        assert_eq!(
            journal
                .record("plugin.frame", serde_json::json!({"n": 1}))
                .expect("first"),
            1
        );
        assert_eq!(
            journal.finish(outcome()).expect("finish").events_recorded,
            1
        );
        let event = fs::read_to_string(journal.event_path()).expect("event file");
        let parsed: TraceEvent = serde_json::from_str(event.trim()).expect("event json");
        assert_eq!(parsed.seq, 1);
        assert!(journal.summary_path().is_file());
        assert!(matches!(
            journal.record("late", Value::Null),
            Err(JournalError::AlreadyFinished)
        ));
    }

    #[test]
    fn degraded_journal_keeps_correlation_without_claiming_persistence() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::degraded(&workspace);
        assert!(!journal.is_durable());
        journal
            .record("runtime.message", json!({"code":"diagnostic.degraded"}))
            .expect("in-memory record");
        let summary = journal.finish(outcome()).expect("in-memory summary");
        assert_eq!(summary.events_recorded, 0);
        assert!(summary.run_id.starts_with("run-unpersisted-"));
        assert!(!journal.event_path().exists());
        assert!(!journal.summary_path().exists());
    }

    #[test]
    fn state_transitions_have_the_complete_decision_contract() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        journal
            .record_transition(
                "ready",
                TransitionTrigger::new(TriggerKind::Request, "tactus.test", "test.requested")
                    .with_details(json!({"subject":"unicode-你好"})),
                TransitionGuard::new(
                    "request is valid",
                    true,
                    "The typed request passed validation.",
                ),
                "running",
                Presentation::new(PresentationCategory::State, "测试任务已开始"),
            )
            .expect("transition");
        journal.finish(outcome()).expect("finish");

        let encoded = fs::read_to_string(journal.event_path()).expect("event file");
        let event: TraceEvent = serde_json::from_str(encoded.trim()).expect("trace event");
        assert_eq!(event.kind, "runtime.state_transition");
        assert_eq!(event.data["state_before"], "ready");
        assert_eq!(event.data["trigger"]["kind"], "request");
        assert_eq!(event.data["trigger"]["source"], "tactus.test");
        assert_eq!(event.data["trigger"]["code"], "test.requested");
        assert_eq!(event.data["guard"]["condition"], "request is valid");
        assert_eq!(event.data["guard"]["passed"], true);
        assert_eq!(
            event.data["guard"]["reason"],
            "The typed request passed validation."
        );
        assert_eq!(event.data["state_after"], "running");
        assert_eq!(
            event.presentation.expect("presentation").message,
            "测试任务已开始"
        );
    }

    #[test]
    fn durable_diagnostics_redact_provider_content_and_terminal_values() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        journal
            .record(
                "provider.evidence",
                json!({
                    "provider":"claude-code",
                    "prompt":"DO_NOT_PERSIST_PROMPT",
                    "raw":{"secret":"DO_NOT_PERSIST_RAW"},
                    "report":{
                        "summary":{
                            "outcome":{
                                "stderr":"DO_NOT_PERSIST_NESTED_STDERR",
                                "error":"DO_NOT_PERSIST_NESTED_ERROR"
                            }
                        }
                    }
                }),
            )
            .expect("event");
        let mut terminal_outcome = outcome();
        terminal_outcome.terminal = Some(crate::protocol::TerminalResult::Success {
            value: json!({"text":"DO_NOT_PERSIST_RESULT"}),
        });
        let live = journal.finish(terminal_outcome).expect("finish");
        assert_eq!(
            live.outcome.terminal,
            Some(crate::protocol::TerminalResult::Success {
                value: json!({"text":"DO_NOT_PERSIST_RESULT"})
            })
        );

        let durable = format!(
            "{}\n{}",
            fs::read_to_string(journal.event_path()).expect("events"),
            fs::read_to_string(journal.summary_path()).expect("summary")
        );
        assert!(!durable.contains("DO_NOT_PERSIST_PROMPT"));
        assert!(!durable.contains("DO_NOT_PERSIST_RAW"));
        assert!(!durable.contains("DO_NOT_PERSIST_RESULT"));
        assert!(!durable.contains("DO_NOT_PERSIST_NESTED_STDERR"));
        assert!(!durable.contains("DO_NOT_PERSIST_NESTED_ERROR"));
        assert!(durable.contains("sha256"));
    }

    #[test]
    fn durable_failure_summaries_preserve_only_public_codes_and_not_external_text() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        let mut failure = outcome();
        failure.kind = InvocationKind::PluginFailed;
        failure.exit_code = Some(1);
        failure.error = Some("DO_NOT_PERSIST_RUNTIME_ERROR".to_owned());
        failure.observation_error = Some("DO_NOT_PERSIST_OBSERVER_ERROR".to_owned());
        failure.terminal = Some(crate::protocol::TerminalResult::Failure {
            error: crate::protocol::PluginFailure {
                code: "stable.failure.code".to_owned(),
                message: "DO_NOT_PERSIST_FAILURE_MESSAGE".to_owned(),
                details: Some(json!({"unknown_field":"DO_NOT_PERSIST_FAILURE_DETAILS"})),
            },
        });
        journal.finish(failure).expect("finish");

        let durable = format!(
            "{}\n{}",
            fs::read_to_string(journal.event_path()).expect("events"),
            fs::read_to_string(journal.summary_path()).expect("summary")
        );
        assert!(!durable.contains("stable.failure.code"));
        assert!(durable.contains("invalid_identifier."));
        for secret in [
            "DO_NOT_PERSIST_RUNTIME_ERROR",
            "DO_NOT_PERSIST_OBSERVER_ERROR",
            "DO_NOT_PERSIST_FAILURE_MESSAGE",
            "DO_NOT_PERSIST_FAILURE_DETAILS",
        ] {
            assert!(!durable.contains(secret), "persisted {secret}");
        }
        assert!(durable.contains("sha256"));
    }

    #[test]
    fn arbitrary_identifier_shaped_plugin_values_are_fingerprinted() {
        let value = redact_diagnostic_value(json!({
            "code":"DO_NOT_PERSIST_CODE",
            "event_type":"DO_NOT_PERSIST_EVENT_TYPE",
            "id":"DO_NOT_PERSIST_ID",
            "method":"DO_NOT_PERSIST_METHOD",
            "type":"DO_NOT_PERSIST_TYPE",
            "nested":{"code":"outcome_unknown", "method":"invoke"}
        }));
        let encoded = serde_json::to_string(&value).expect("diagnostic JSON");
        for secret in [
            "DO_NOT_PERSIST_CODE",
            "DO_NOT_PERSIST_EVENT_TYPE",
            "DO_NOT_PERSIST_ID",
            "DO_NOT_PERSIST_METHOD",
            "DO_NOT_PERSIST_TYPE",
        ] {
            assert!(!encoded.contains(secret), "persisted {secret}");
        }
        assert!(encoded.contains("outcome_unknown"));
        assert!(encoded.contains("invoke"));
        assert!(encoded.contains("sha256"));
    }

    #[test]
    fn journal_rejects_a_contradictory_terminal_summary() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        let mut contradictory = outcome();
        contradictory.exit_code = Some(1);
        assert!(matches!(
            journal.finish(contradictory),
            Err(JournalError::InvalidOutcome(_))
        ));
        assert!(!journal.summary_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn journal_create_rejects_a_linked_runs_directory() {
        use std::os::unix::fs::symlink;

        let temporary = tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        let outside = temporary.path().join("outside-runs");
        fs::create_dir_all(root.join(".tactus")).expect("control");
        fs::create_dir(&outside).expect("outside runs");
        symlink(&outside, root.join(".tactus/runs")).expect("runs symlink");
        let workspace = Workspace::at(&root);

        assert!(matches!(
            RunJournal::create(&workspace),
            Err(JournalError::UnsafePath(".tactus/runs"))
        ));
        assert_eq!(fs::read_dir(&outside).expect("outside contents").count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn journal_create_rejects_a_runs_junction() {
        let temporary = tempdir().expect("temporary directory");
        let root = temporary.path().join("project");
        let outside = temporary.path().join("outside-runs");
        fs::create_dir_all(root.join(".tactus")).expect("control");
        fs::create_dir(&outside).expect("outside runs");
        let runs = root.join(".tactus/runs");
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:TACTUS_TEST_JUNCTION -Target $env:TACTUS_TEST_TARGET | Out-Null",
            ])
            .env("TACTUS_TEST_JUNCTION", &runs)
            .env("TACTUS_TEST_TARGET", &outside)
            .output()
            .expect("create junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let workspace = Workspace::at(&root);

        assert!(matches!(
            RunJournal::create(&workspace),
            Err(JournalError::UnsafePath(".tactus/runs"))
        ));
        assert_eq!(fs::read_dir(&outside).expect("outside contents").count(), 0);
    }

    #[test]
    fn typed_failure_projection_keeps_only_validated_reconciliation_evidence() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        let mut failure = outcome();
        failure.kind = InvocationKind::PluginFailed;
        failure.exit_code = Some(1);
        failure.terminal = Some(crate::protocol::TerminalResult::Failure {
            error: crate::protocol::PluginFailure {
                code: "outcome_unknown".to_owned(),
                message: "DO_NOT_PERSIST_PROVIDER_TEXT".to_owned(),
                details: Some(json!({
                    "provider":"claude-code",
                    "phase":"partial_output",
                    "occurrence_id":"occ:fixture",
                    "business_key_sha256":"ab".repeat(32),
                    "external_effect_possible":true,
                    "reconciliation":["Inspect the external system."],
                    "validation_failed":[{
                        "stage":"structure",
                        "rule":"missing_subquestion",
                        "severity":"Correctness",
                        "expected":{"parts":["a","b","c"]},
                        "observed":{"parts":["b","c"],"message":"DO_NOT_PERSIST_VALIDATOR_MESSAGE"},
                        "provenance":{"source":"rubric", "path":"DO_NOT_PERSIST_PATH"}
                    }],
                    "text":"DO_NOT_PERSIST_RESULT_TEXT"
                })),
            },
        });
        let details = match failure.terminal.as_ref() {
            Some(crate::protocol::TerminalResult::Failure { error }) => {
                error.details.as_ref().expect("details")
            }
            _ => unreachable!("fixture is a failure"),
        };
        journal
            .record("runtime.typed_failure", diagnostic_failure_details(details))
            .expect("typed diagnostic");
        journal.finish(failure).expect("finish");

        let durable = format!(
            "{}\n{}",
            fs::read_to_string(journal.event_path()).expect("events"),
            fs::read_to_string(journal.summary_path()).expect("summary")
        );
        for evidence in [
            "partial_output",
            "external_effect_possible",
            "missing_subquestion",
            "structure",
            "Correctness",
        ] {
            assert!(durable.contains(evidence), "missing {evidence}");
        }
        for secret in [
            "claude-code",
            "occ:fixture",
            "Inspect the external system.",
            "parts",
            "DO_NOT_PERSIST_PROVIDER_TEXT",
            "DO_NOT_PERSIST_RESULT_TEXT",
            "DO_NOT_PERSIST_VALIDATOR_MESSAGE",
            "DO_NOT_PERSIST_PATH",
        ] {
            assert!(!durable.contains(secret), "persisted {secret}");
        }
    }
}
