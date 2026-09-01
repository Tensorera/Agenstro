//! Bounded, redacted projections for desktop and other control-plane clients.
//!
//! The on-disk workspace and trace layouts remain runtime implementation
//! details.  Consumers use these versioned DTOs instead of parsing TOML,
//! sorting scripts, or walking `.tactus/runs` themselves.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::{
    journal::{Presentation, RunSummary, TRACE_API, TraceEvent},
    limits::MAX_FRAME_BYTES_CEILING,
    outcome::{classify_outcome, validate_outcome_consistency},
    process::{InvocationKind, ProcessOutcome},
    workspace::{RuntimeConfig, Workspace, WorkspaceError, discover_scripts, doctor},
};

/// Version of the machine-readable control envelope.
pub const CONTROL_API: &str = "tactus.control/v1";
/// Version of Motivo's read-only runtime projection.
pub const STUDIO_API: &str = "agenstro.studio/v1";

const MAX_RUN_LIMIT: usize = 200;
const MAX_EVENT_LIMIT: usize = 1_000;
const MAX_EVENT_LINE_BYTES: usize = MAX_FRAME_BYTES_CEILING + 4 * 1024 * 1024;
const MAX_EVENT_PAGE_BYTES: usize = MAX_EVENT_LINE_BYTES;
const MAX_EVENT_SCAN_BYTES: usize = 128 * 1024 * 1024;
const MAX_SUMMARY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RUN_DIRECTORIES_SCANNED: usize = 2_000;
const MAX_RUN_DIRECTORY_ENTRIES_VISITED: usize = 100_000;
const MAX_DIAGNOSTIC_CHARACTERS: usize = 4_096;

/// A successful machine-control response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSuccess<T> {
    /// Control protocol version.
    pub api: &'static str,
    /// Stable command name.
    pub command: &'static str,
    /// Indicates that a domain result was obtained.
    pub status: &'static str,
    /// Command-specific payload.
    pub data: T,
}

impl<T> ControlSuccess<T> {
    /// Wrap one successful control result.
    #[must_use]
    pub fn new(command: &'static str, data: T) -> Self {
        Self {
            api: CONTROL_API,
            command,
            status: "completed",
            data,
        }
    }
}

/// A failed machine-control response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlFailure {
    /// Control protocol version.
    pub api: &'static str,
    /// Stable command name.
    pub command: &'static str,
    /// Always `error`.
    pub status: &'static str,
    /// Stable structured failure.
    pub error: StudioFailure,
}

impl ControlFailure {
    /// Convert an internal studio error to a stable response.
    #[must_use]
    pub fn new(command: &'static str, error: &StudioError) -> Self {
        Self {
            api: CONTROL_API,
            command,
            status: "error",
            error: StudioFailure {
                code: error.code(),
                message: error.public_message(),
            },
        }
    }
}

/// Stable control failure exposed to clients.
#[derive(Debug, Serialize)]
pub struct StudioFailure {
    /// Machine-readable category.
    pub code: &'static str,
    /// Bounded human-readable diagnostic.
    pub message: String,
}

/// Complete redacted workspace projection.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioSnapshot {
    /// Projection schema version.
    pub api: &'static str,
    /// Snapshot time, encoded as decimal text for JavaScript consumers.
    pub generated_at_unix_ms: String,
    /// Non-sensitive workspace identity.
    pub workspace: StudioWorkspace,
    /// Factual doctor checks.
    pub health: StudioHealth,
    /// Deterministically ordered Haskell sources.
    pub scripts: Vec<StudioScript>,
    /// Typed plugin registries without commands, options, or prompt text.
    pub registries: StudioRegistries,
    /// Most recent invocation traces.
    pub runs: Vec<StudioRun>,
}

/// Workspace identity safe to expose to an unprivileged renderer.
#[derive(Debug, Serialize)]
pub struct StudioWorkspace {
    /// Final path component, not an absolute host path.
    pub name: String,
}

/// Aggregate and individual runtime checks.
#[derive(Debug, Serialize)]
pub struct StudioHealth {
    /// True only when every check succeeded.
    pub ok: bool,
    /// Ordered factual checks.
    pub checks: Vec<StudioCheck>,
}

/// One doctor result.
#[derive(Debug, Serialize)]
pub struct StudioCheck {
    /// Stable check name.
    pub name: String,
    /// Whether it passed.
    pub ok: bool,
    /// Bounded diagnostic evidence.
    pub detail: String,
}

/// One Haskell source discovered by Tactus.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioScript {
    /// Forward-slash workspace-relative path.
    pub relative_path: String,
    /// Three-digit entry order; helpers omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u16>,
    /// Whether Tactus runs this file as an entry point.
    pub runnable: bool,
}

/// All three typed registries.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioRegistries {
    /// Provider used when no provider is selected.
    pub default_provider: String,
    /// Provider-shaped plugins.
    pub providers: Vec<StudioPlugin>,
    /// Effect-shaped plugins.
    pub effects: Vec<StudioPlugin>,
    /// Open generic plugins.
    pub plugins: Vec<StudioPlugin>,
}

/// Redacted plugin inventory entry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioPlugin {
    /// Registry key.
    pub name: String,
    /// `provider`, `effect`, or `plugin`.
    pub namespace: &'static str,
    /// Whether doctor resolved the configured executable.
    pub available: bool,
    /// Whether this is the default provider.
    pub default: bool,
    /// Optional provider model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional provider effort.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Whether an effect observes other invocations.
    pub observes_invocations: bool,
}

/// Compact projection of one run directory.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioRun {
    /// Opaque, path-safe identifier.
    pub run_id: String,
    /// `open`, `corrupt`, `succeeded`, `failed`, or `outcome_unknown`.
    pub state: String,
    /// Trace integrity seen while projecting the run.
    pub integrity: StudioIntegrity,
    /// Start time as decimal milliseconds.
    pub started_unix_ms: String,
    /// Finish time as decimal milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_unix_ms: Option<String>,
    /// Number of events claimed by a terminal summary.
    pub events_recorded: String,
    /// Human-readable, non-secret subject.
    pub label: String,
    /// Registry namespace when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Plugin/provider/effect name when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Plugin method when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Compact terminal outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<StudioOutcome>,
}

/// Bounded terminal outcome shown by Studio.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioOutcome {
    /// Normalized invocation kind.
    pub kind: String,
    /// Leader exit code when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Bounded runtime/protocol diagnostic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Elapsed milliseconds as decimal text.
    pub elapsed_ms: String,
    /// Whether diagnostic stderr was truncated by Tactus.
    pub stderr_truncated: bool,
}

/// Trace integrity visible to a control client.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudioIntegrity {
    /// Every observed record was valid.
    Ok,
    /// The final line is still being appended or a read budget was reached.
    Partial,
    /// A complete record contradicted the trace contract.
    Corrupt,
}

/// One open trace event with JavaScript-safe counters.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioEvent {
    /// One-based sequence encoded as decimal text.
    pub seq: String,
    /// Event time encoded as decimal text.
    pub at_unix_ms: String,
    /// Open event category.
    pub kind: String,
    /// Ready-to-display human projection supplied by Tactus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    /// Open structured event payload.
    pub data: Value,
}

/// Bounded event page for one run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioEventPage {
    /// Projection schema version.
    pub api: &'static str,
    /// Current compact run state.
    pub run: StudioRun,
    /// Events after the requested sequence.
    pub events: Vec<StudioEvent>,
    /// Highest sequence returned, or the caller's input when empty.
    pub next_after: String,
    /// True only when a terminal summary exists and all current events were read.
    pub complete: bool,
    /// Page-level integrity.
    pub integrity: StudioIntegrity,
    /// Redacted terminal summary when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<StudioSummary>,
}

/// Redacted terminal summary.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioSummary {
    /// Journal creation time as decimal text.
    pub started_unix_ms: String,
    /// Summary publication time as decimal text.
    pub finished_unix_ms: String,
    /// Number of journal events as decimal text.
    pub events_recorded: String,
    /// Compact process result.
    pub outcome: StudioOutcome,
}

/// Build a bounded, redacted snapshot for a Studio client.
pub fn inspect(start: &Path, run_limit: usize) -> Result<StudioSnapshot, StudioError> {
    inspect_with_root_policy(start, run_limit, false)
}

/// Build a snapshot only when `start` is exactly the discovered workspace root.
pub fn inspect_exact(start: &Path, run_limit: usize) -> Result<StudioSnapshot, StudioError> {
    inspect_with_root_policy(start, run_limit, true)
}

fn inspect_with_root_policy(
    start: &Path,
    run_limit: usize,
    exact_root: bool,
) -> Result<StudioSnapshot, StudioError> {
    if !(1..=MAX_RUN_LIMIT).contains(&run_limit) {
        return Err(StudioError::InvalidLimit(format!(
            "run limit must be between 1 and {MAX_RUN_LIMIT}"
        )));
    }
    let workspace = Workspace::discover(start)?;
    if exact_root {
        let supplied = dunce::canonicalize(start).map_err(StudioError::Io)?;
        let discovered = dunce::canonicalize(&workspace.root).map_err(StudioError::Io)?;
        if supplied != discovered {
            return Err(StudioError::WorkspaceRootMismatch);
        }
    }
    let config = workspace.load_config()?;
    let checks = doctor(&workspace);
    let availability = checks
        .iter()
        .map(|check| (check.name.clone(), check.ok))
        .collect::<BTreeMap<_, _>>();
    let health = StudioHealth {
        ok: checks.iter().all(|check| check.ok),
        checks: checks
            .into_iter()
            .map(|check| StudioCheck {
                detail: redacted_check_detail(&check.name, check.ok),
                name: check.name,
                ok: check.ok,
            })
            .collect(),
    };
    let scripts = discover_scripts(&workspace)?
        .into_iter()
        .map(|script| StudioScript {
            relative_path: script.relative_path,
            order: script.order,
            runnable: script.runnable,
        })
        .collect();
    let registries = project_registries(&config, &availability);
    let runs = recent_runs(&workspace, run_limit)?;
    let name = workspace
        .root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("workspace")
        .to_owned();
    Ok(StudioSnapshot {
        api: STUDIO_API,
        generated_at_unix_ms: unix_millis().to_string(),
        workspace: StudioWorkspace { name },
        health,
        scripts,
        registries,
        runs,
    })
}

/// Read a bounded, validated page of one run's trace events.
pub fn events(
    start: &Path,
    run_id: &str,
    after: u64,
    limit: usize,
    max_bytes: usize,
) -> Result<StudioEventPage, StudioError> {
    if !(1..=MAX_EVENT_LIMIT).contains(&limit) {
        return Err(StudioError::InvalidLimit(format!(
            "event limit must be between 1 and {MAX_EVENT_LIMIT}"
        )));
    }
    if !(1..=MAX_EVENT_PAGE_BYTES).contains(&max_bytes) {
        return Err(StudioError::InvalidLimit(format!(
            "event byte limit must be between 1 and {MAX_EVENT_PAGE_BYTES}"
        )));
    }
    validate_run_id(run_id)?;
    let workspace = Workspace::discover(start)?;
    let run_path = workspace.runs_path.join(run_id);
    if !require_contained_run_directory(&workspace.runs_path, &run_path)? {
        return Err(StudioError::RunNotFound(run_id.to_owned()));
    }
    let summary = read_summary(&run_path, run_id)?;
    let run = project_run(&run_path, run_id, summary.as_ref());
    let event_path = run_path.join("events.jsonl");
    require_plain_file(&event_path)?;
    let file = File::open(&event_path).map_err(StudioError::Io)?;
    let mut reader = BufReader::new(file);
    let mut scanned_bytes = 0usize;
    let mut retained_bytes = 0usize;
    let mut expected = 1u64;
    let mut page = Vec::new();
    let mut next_after = after;
    let mut reached_eof = false;
    let mut integrity = StudioIntegrity::Ok;

    loop {
        if page.len() == limit {
            break;
        }
        let line = read_bounded_line(&mut reader, MAX_EVENT_LINE_BYTES)?;
        let Some(line) = line else {
            reached_eof = true;
            break;
        };
        scanned_bytes = scanned_bytes.saturating_add(line.bytes.len());
        if scanned_bytes > MAX_EVENT_SCAN_BYTES {
            return Err(StudioError::BudgetExceeded(event_path));
        }
        if !line.terminated {
            integrity = StudioIntegrity::Partial;
            break;
        }
        let trace: TraceEvent = match serde_json::from_slice(strip_newline(&line.bytes)) {
            Ok(trace) => trace,
            Err(_) => {
                integrity = StudioIntegrity::Corrupt;
                break;
            }
        };
        if trace.api != TRACE_API || trace.run_id != run_id || trace.seq != expected {
            integrity = StudioIntegrity::Corrupt;
            break;
        }
        expected = expected.saturating_add(1);
        if trace.seq <= after {
            continue;
        }
        if retained_bytes.saturating_add(line.bytes.len()) > max_bytes {
            if page.is_empty() {
                return Err(StudioError::InvalidLimit(
                    "the next event exceeds maxBytes; increase the event byte limit".to_owned(),
                ));
            }
            integrity = StudioIntegrity::Partial;
            break;
        }
        retained_bytes = retained_bytes.saturating_add(line.bytes.len());
        next_after = trace.seq;
        page.push(StudioEvent {
            seq: trace.seq.to_string(),
            at_unix_ms: trace.at_unix_ms.to_string(),
            kind: trace.kind,
            presentation: trace.presentation,
            data: trace.data,
        });
    }

    if let Some(summary) = summary.as_ref()
        && reached_eof
        && summary.events_recorded.saturating_add(1) != expected
    {
        integrity = StudioIntegrity::Corrupt;
    }
    let complete = reached_eof
        && summary.is_some()
        && matches!(integrity, StudioIntegrity::Ok)
        && summary
            .as_ref()
            .is_some_and(|summary| summary.events_recorded < expected);
    Ok(StudioEventPage {
        api: STUDIO_API,
        run,
        events: page,
        next_after: next_after.to_string(),
        complete,
        integrity,
        summary: summary.as_ref().map(project_summary),
    })
}

fn project_registries(
    config: &RuntimeConfig,
    availability: &BTreeMap<String, bool>,
) -> StudioRegistries {
    let providers = config
        .providers
        .iter()
        .map(|(name, provider)| StudioPlugin {
            name: name.clone(),
            namespace: "provider",
            available: availability
                .get(format!("provider-native:{name}").as_str())
                .or_else(|| availability.get(format!("provider:{name}").as_str()))
                .copied()
                .unwrap_or(false),
            default: *name == config.default_provider,
            model: provider.model.clone(),
            effort: provider.effort.clone(),
            observes_invocations: false,
        })
        .collect();
    let effects = config
        .effects
        .iter()
        .map(|(name, effect)| StudioPlugin {
            name: name.clone(),
            namespace: "effect",
            available: availability
                .get(format!("effect:{name}").as_str())
                .copied()
                .unwrap_or(false),
            default: false,
            model: None,
            effort: None,
            observes_invocations: effect.observe_invocations,
        })
        .collect();
    let plugins = config
        .plugins
        .keys()
        .map(|name| StudioPlugin {
            name: name.clone(),
            namespace: "plugin",
            available: availability
                .get(format!("plugin:{name}").as_str())
                .copied()
                .unwrap_or(false),
            default: false,
            model: None,
            effort: None,
            observes_invocations: false,
        })
        .collect();
    StudioRegistries {
        default_provider: config.default_provider.clone(),
        providers,
        effects,
        plugins,
    }
}

fn recent_runs(workspace: &Workspace, limit: usize) -> Result<Vec<StudioRun>, StudioError> {
    let mut directories = BTreeMap::new();
    for (index, entry) in fs::read_dir(&workspace.runs_path)
        .map_err(StudioError::Io)?
        .enumerate()
    {
        if index >= MAX_RUN_DIRECTORY_ENTRIES_VISITED {
            return Err(StudioError::BudgetExceeded(workspace.runs_path.clone()));
        }
        let Ok(entry) = entry else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if validate_run_id(&name).is_err() {
            continue;
        }
        retain_newest_run_directory(&mut directories, name, entry.path());
    }
    directories
        .into_iter()
        .rev()
        .take(limit)
        .map(
            |(run_id, path)| match require_contained_run_directory(&workspace.runs_path, &path) {
                Ok(true) => match read_summary(&path, &run_id) {
                    Ok(summary) => Ok(project_run(&path, &run_id, summary.as_ref())),
                    Err(
                        StudioError::Json(_)
                        | StudioError::CorruptTrace(_)
                        | StudioError::BudgetExceeded(_)
                        | StudioError::UnsafeTracePath(_),
                    ) => {
                        let mut run = project_run(&path, &run_id, None);
                        run.state = "corrupt".to_owned();
                        run.integrity = StudioIntegrity::Corrupt;
                        Ok(run)
                    }
                    Err(error) => Err(error),
                },
                Ok(false) | Err(StudioError::UnsafeTracePath(_)) => Ok(project_unsafe_run(&run_id)),
                Err(error) => Err(error),
            },
        )
        .collect()
}

fn retain_newest_run_directory(
    directories: &mut BTreeMap<String, PathBuf>,
    run_id: String,
    path: PathBuf,
) {
    directories.insert(run_id, path);
    if directories.len() > MAX_RUN_DIRECTORIES_SCANNED
        && let Some(oldest) = directories.keys().next().cloned()
    {
        directories.remove(&oldest);
    }
}

fn project_unsafe_run(run_id: &str) -> StudioRun {
    StudioRun {
        run_id: run_id.to_owned(),
        state: "corrupt".to_owned(),
        integrity: StudioIntegrity::Corrupt,
        started_unix_ms: run_id_timestamp(run_id).unwrap_or(0).to_string(),
        finished_unix_ms: None,
        events_recorded: "0".to_owned(),
        label: "Unsafe run journal".to_owned(),
        namespace: None,
        subject: None,
        method: None,
        outcome: None,
    }
}

fn project_run(path: &Path, run_id: &str, summary: Option<&RunSummary>) -> StudioRun {
    let context = first_event_context(path, run_id);
    let summary_integrity = summary.map_or(StudioIntegrity::Partial, |_| StudioIntegrity::Ok);
    let (state, started, finished, events_recorded, outcome) = summary.map_or_else(
        || {
            (
                "open".to_owned(),
                run_id_timestamp(run_id).unwrap_or(0),
                None,
                0,
                None,
            )
        },
        |summary| {
            (
                classify_outcome(&summary.outcome).as_str().to_owned(),
                summary.started_unix_ms,
                Some(summary.finished_unix_ms),
                summary.events_recorded,
                Some(project_outcome(&summary.outcome)),
            )
        },
    );
    StudioRun {
        run_id: run_id.to_owned(),
        state,
        integrity: if context.corrupt {
            StudioIntegrity::Corrupt
        } else {
            summary_integrity
        },
        started_unix_ms: started.to_string(),
        finished_unix_ms: finished.map(|value| value.to_string()),
        events_recorded: events_recorded.to_string(),
        label: context
            .label
            .unwrap_or_else(|| "Runtime invocation".to_owned()),
        namespace: context.namespace,
        subject: context.subject,
        method: context.method,
        outcome,
    }
}

fn project_summary(summary: &RunSummary) -> StudioSummary {
    StudioSummary {
        started_unix_ms: summary.started_unix_ms.to_string(),
        finished_unix_ms: summary.finished_unix_ms.to_string(),
        events_recorded: summary.events_recorded.to_string(),
        outcome: project_outcome(&summary.outcome),
    }
}

fn project_outcome(outcome: &ProcessOutcome) -> StudioOutcome {
    StudioOutcome {
        kind: invocation_kind_name(outcome.kind).to_owned(),
        exit_code: outcome.exit_code,
        error: outcome.error.as_deref().map(truncate_diagnostic),
        elapsed_ms: outcome.elapsed_ms.to_string(),
        stderr_truncated: outcome.stderr_truncated,
    }
}

fn invocation_kind_name(kind: InvocationKind) -> &'static str {
    match kind {
        InvocationKind::Succeeded => "succeeded",
        InvocationKind::PluginFailed => "plugin_failed",
        InvocationKind::ProcessFailed => "process_failed",
        InvocationKind::ProtocolFailed => "protocol_failed",
        InvocationKind::RuntimeFailed => "runtime_failed",
        InvocationKind::DeadlineExceeded => "deadline_exceeded",
        InvocationKind::Cancelled => "cancelled",
    }
}

#[derive(Default)]
struct RunContext {
    label: Option<String>,
    namespace: Option<String>,
    subject: Option<String>,
    method: Option<String>,
    corrupt: bool,
}

fn first_event_context(path: &Path, run_id: &str) -> RunContext {
    let event_path = path.join("events.jsonl");
    if require_plain_file(&event_path).is_err() {
        return RunContext {
            corrupt: true,
            ..RunContext::default()
        };
    }
    let Ok(file) = File::open(event_path) else {
        return RunContext {
            corrupt: true,
            ..RunContext::default()
        };
    };
    let mut reader = BufReader::new(file);
    let line = match read_bounded_line(&mut reader, MAX_EVENT_LINE_BYTES) {
        Ok(Some(line)) => line,
        Ok(None) => return RunContext::default(),
        Err(_) => {
            return RunContext {
                corrupt: true,
                ..RunContext::default()
            };
        }
    };
    if !line.terminated {
        return RunContext {
            corrupt: false,
            ..RunContext::default()
        };
    }
    let Ok(event) = serde_json::from_slice::<TraceEvent>(strip_newline(&line.bytes)) else {
        return RunContext {
            corrupt: true,
            ..RunContext::default()
        };
    };
    if event.api != TRACE_API || event.run_id != run_id || event.seq != 1 {
        return RunContext {
            corrupt: true,
            ..RunContext::default()
        };
    }
    let object = event.data.as_object();
    let value = |name: &str| {
        object
            .and_then(|data| data.get(name))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    };
    match event.kind.as_str() {
        "runtime.state_transition" => {
            let trigger = object
                .and_then(|data| data.get("trigger"))
                .and_then(Value::as_object);
            let code = trigger
                .and_then(|trigger| trigger.get("code"))
                .and_then(Value::as_str);
            let details = trigger
                .and_then(|trigger| trigger.get("details"))
                .and_then(Value::as_object);
            let detail = |name: &str| {
                details
                    .and_then(|details| details.get(name))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            };
            if code == Some("workflow.generation_requested") {
                let provider = detail("provider");
                return RunContext {
                    label: Some(provider.as_ref().map_or_else(
                        || "Generate workflow".to_owned(),
                        |name| format!("Generate with {name}"),
                    )),
                    namespace: Some("provider".to_owned()),
                    subject: provider,
                    method: Some("generate".to_owned()),
                    corrupt: false,
                };
            }
            let namespace = detail("namespace");
            let subject = detail("plugin");
            let method = detail("method");
            let label = match (&namespace, &subject, &method) {
                (Some(namespace), Some(subject), Some(method)) => {
                    format!("{namespace}:{subject} · {method}")
                }
                _ => "Runtime transition".to_owned(),
            };
            RunContext {
                label: Some(label),
                namespace,
                subject,
                method,
                corrupt: false,
            }
        }
        "generation.started" => {
            let provider = value("provider");
            RunContext {
                label: Some(provider.as_ref().map_or_else(
                    || "Generate workflow".to_owned(),
                    |name| format!("Generate with {name}"),
                )),
                namespace: Some("provider".to_owned()),
                subject: provider,
                method: Some("generate".to_owned()),
                corrupt: false,
            }
        }
        "invocation.started" | "dispatch.started" => {
            let namespace = value("namespace");
            let subject = value("plugin");
            let method = value("method");
            let label = match (&namespace, &subject, &method) {
                (Some(namespace), Some(subject), Some(method)) => {
                    format!("{namespace}:{subject} · {method}")
                }
                _ => "Plugin invocation".to_owned(),
            };
            RunContext {
                label: Some(label),
                namespace,
                subject,
                method,
                corrupt: false,
            }
        }
        _ => RunContext {
            label: Some(event.kind),
            corrupt: false,
            ..RunContext::default()
        },
    }
}

fn read_summary(path: &Path, run_id: &str) -> Result<Option<RunSummary>, StudioError> {
    let summary_path = path.join("summary.json");
    match fs::symlink_metadata(&summary_path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(StudioError::UnsafeTracePath(summary_path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(StudioError::Io(error)),
    }
    let bytes = read_bounded_file(&summary_path, MAX_SUMMARY_BYTES)?;
    let summary: RunSummary = serde_json::from_slice(&bytes).map_err(StudioError::Json)?;
    if summary.api != TRACE_API || summary.run_id != run_id {
        return Err(StudioError::CorruptTrace(run_id.to_owned()));
    }
    validate_outcome_consistency(&summary.outcome)
        .map_err(|_| StudioError::CorruptTrace(run_id.to_owned()))?;
    Ok(Some(summary))
}

fn read_bounded_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, StudioError> {
    require_plain_file(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(StudioError::Io)?
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(StudioError::Io)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(StudioError::BudgetExceeded(path.to_path_buf()));
    }
    Ok(bytes)
}

fn require_contained_run_directory(root: &Path, path: &Path) -> Result<bool, StudioError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir()
                || metadata.file_type().is_symlink()
                || metadata_is_reparse_point(&metadata)
            {
                return Err(StudioError::UnsafeTracePath(path.to_path_buf()));
            }
            let canonical_root = dunce::canonicalize(root).map_err(StudioError::Io)?;
            let canonical = dunce::canonicalize(path).map_err(StudioError::Io)?;
            if canonical.parent() != Some(canonical_root.as_path()) {
                return Err(StudioError::UnsafeTracePath(path.to_path_buf()));
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(StudioError::Io(error)),
    }
}

fn require_plain_file(path: &Path) -> Result<(), StudioError> {
    let metadata = fs::symlink_metadata(path).map_err(StudioError::Io)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(StudioError::UnsafeTracePath(path.to_path_buf()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| StudioError::UnsafeTracePath(path.to_path_buf()))?;
    let canonical_parent = dunce::canonicalize(parent).map_err(StudioError::Io)?;
    let canonical = dunce::canonicalize(path).map_err(StudioError::Io)?;
    if canonical.parent() != Some(canonical_parent.as_path()) {
        return Err(StudioError::UnsafeTracePath(path.to_path_buf()));
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

struct BoundedLine {
    bytes: Vec<u8>,
    terminated: bool,
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> Result<Option<BoundedLine>, StudioError> {
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(StudioError::Io)?;
        if available.is_empty() {
            return Ok((!output.is_empty()).then_some(BoundedLine {
                bytes: output,
                terminated: false,
            }));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if output.len().saturating_add(take) > max_bytes {
            return Err(StudioError::EventLineTooLarge);
        }
        let terminated = available.get(take.saturating_sub(1)) == Some(&b'\n');
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if terminated {
            return Ok(Some(BoundedLine {
                bytes: output,
                terminated: true,
            }));
        }
    }
}

fn strip_newline(bytes: &[u8]) -> &[u8] {
    let without_lf = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

fn validate_run_id(run_id: &str) -> Result<(), StudioError> {
    let valid = run_id.starts_with("run-")
        && run_id.len() <= 128
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid {
        return Err(StudioError::InvalidRunId(run_id.to_owned()));
    }
    Ok(())
}

fn run_id_timestamp(run_id: &str) -> Option<u64> {
    run_id.strip_prefix("run-")?.split('-').next()?.parse().ok()
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn truncate_diagnostic(value: &str) -> String {
    value.chars().take(MAX_DIAGNOSTIC_CHARACTERS).collect()
}

fn redacted_check_detail(name: &str, ok: bool) -> String {
    if ok {
        match name {
            "config" => "Typed configuration loaded".to_owned(),
            "clef-sdk-link" => "Clef SDK linkage resolved".to_owned(),
            "ghc" | "cabal" => format!("{name} is available"),
            _ => "Configured executable is available".to_owned(),
        }
    } else {
        format!("{name} is unavailable; run `tactus doctor` for host details")
    }
}

/// Failure while projecting Tactus state for Studio.
#[derive(Debug, Error)]
pub enum StudioError {
    /// Workspace discovery or typed configuration failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// A bounded filesystem operation failed.
    #[error("studio filesystem operation failed: {0}")]
    Io(#[source] io::Error),
    /// A complete JSON trace record was malformed.
    #[error("studio trace JSON is invalid: {0}")]
    Json(#[source] serde_json::Error),
    /// Client supplied an invalid limit.
    #[error("{0}")]
    InvalidLimit(String),
    /// A run identifier could escape the runs directory or is malformed.
    #[error("invalid run id {0:?}")]
    InvalidRunId(String),
    /// The selected run does not exist.
    #[error("run {0:?} was not found")]
    RunNotFound(String),
    /// A persisted trace contradicted its version or correlation id.
    #[error("run {0:?} has a corrupt trace")]
    CorruptTrace(String),
    /// A bounded control-plane read was too large.
    #[error("studio read budget exceeded for {0}")]
    BudgetExceeded(PathBuf),
    /// One event exceeded the control-plane frame budget.
    #[error("studio event line exceeds the durable trace line limit")]
    EventLineTooLarge,
    /// A trace component was not a plain file or directory.
    #[error("studio refused a non-plain trace path: {0}")]
    UnsafeTracePath(PathBuf),
    /// Exact-root mode discovered a workspace above the selected folder.
    #[error("the supplied folder is not the discovered workspace root")]
    WorkspaceRootMismatch,
}

impl StudioError {
    fn code(&self) -> &'static str {
        match self {
            Self::Workspace(_) => "workspace_error",
            Self::Io(_) => "studio_io_failed",
            Self::Json(_) | Self::CorruptTrace(_) => "trace_corrupt",
            Self::InvalidLimit(_) => "invalid_limit",
            Self::InvalidRunId(_) => "invalid_run_id",
            Self::RunNotFound(_) => "run_not_found",
            Self::BudgetExceeded(_) | Self::EventLineTooLarge => "read_budget_exceeded",
            Self::UnsafeTracePath(_) => "unsafe_trace_path",
            Self::WorkspaceRootMismatch => "workspace_root_mismatch",
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::Workspace(_) => {
                "The selected folder is not a valid initialized Tactus workspace.".to_owned()
            }
            Self::Io(_) => "Studio could not read the requested runtime data.".to_owned(),
            Self::Json(_) | Self::CorruptTrace(_) => {
                "The selected run contains a corrupt trace record.".to_owned()
            }
            Self::InvalidLimit(message) => message.clone(),
            Self::InvalidRunId(_) => "The supplied run identifier is invalid.".to_owned(),
            Self::RunNotFound(_) => "The selected run was not found.".to_owned(),
            Self::BudgetExceeded(_) | Self::EventLineTooLarge => {
                "The requested trace exceeds Studio's bounded read limits.".to_owned()
            }
            Self::UnsafeTracePath(_) => {
                "Studio refused an unsafe trace filesystem entry.".to_owned()
            }
            Self::WorkspaceRootMismatch => {
                "The selected folder is not the exact Tactus workspace root.".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use tempfile::tempdir;

    use super::*;
    use crate::{journal::RunJournal, process::InvocationKind, workspace::initialize_workspace};

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
            elapsed_ms: 3,
            progress: None,
        }
    }

    #[test]
    fn bounded_line_preserves_unicode_and_partial_tail() {
        let mut input = Cursor::new("你好\u{2028}world\npartial".as_bytes());
        let first = read_bounded_line(&mut input, 128)
            .expect("line")
            .expect("first");
        assert!(first.terminated);
        assert_eq!(strip_newline(&first.bytes), "你好\u{2028}world".as_bytes());
        let second = read_bounded_line(&mut input, 128)
            .expect("tail")
            .expect("partial");
        assert!(!second.terminated);
        assert_eq!(second.bytes, b"partial");
    }

    #[test]
    fn inspect_redacts_commands_options_and_absolute_script_paths() {
        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        initialize_workspace(&project, Some(&sdk)).expect("workspace");
        fs::write(
            project.join(".tactus/scripts/010_first.hs"),
            "main = pure ()\n",
        )
        .expect("script");
        let config_path = project.join(".tactus/tactus.toml");
        let config = fs::read_to_string(&config_path).expect("config").replace(
            "[providers.codex]",
            "[providers.codex]\nmodel = \"model-secret\"\noptions = { token = \"never-project\" }",
        );
        fs::write(config_path, config).expect("config");

        let snapshot = inspect(&project, 10).expect("snapshot");
        let encoded = serde_json::to_string(&snapshot).expect("json");
        assert!(encoded.contains("010_first.hs"));
        assert!(encoded.contains("model-secret"));
        assert!(!encoded.contains("never-project"));
        assert!(!encoded.contains("clef-sdk.cabal"));
        assert!(!encoded.contains(&project.display().to_string()));
    }

    #[test]
    fn event_page_uses_decimal_strings_and_validates_sequence() {
        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        let report = initialize_workspace(&project, Some(&sdk)).expect("workspace");
        let mut journal = RunJournal::create(&report.workspace).expect("journal");
        journal
            .record_transition(
                "ready",
                crate::journal::TransitionTrigger::new(
                    crate::journal::TriggerKind::Request,
                    "tactus.test",
                    "plugin.invocation_requested",
                )
                .with_details(serde_json::json!({
                    "namespace":"plugin",
                    "plugin":"demo",
                    "method":"work"
                })),
                crate::journal::TransitionGuard::new(
                    "request is valid",
                    true,
                    "The test request passed validation.",
                ),
                "running",
                crate::journal::Presentation::new(
                    crate::journal::PresentationCategory::State,
                    "Demo work started.",
                ),
            )
            .expect("event one");
        journal
            .record("custom.future.event", serde_json::json!({"value":42}))
            .expect("event two");
        let run_id = journal.run_id().to_owned();
        journal.finish(outcome()).expect("summary");

        let first = events(&project, &run_id, 0, 1, 1024 * 1024).expect("first page");
        assert_eq!(first.events[0].seq, "1");
        assert_eq!(
            first.events[0]
                .presentation
                .as_ref()
                .expect("presentation")
                .message,
            "Demo work started."
        );
        assert_eq!(first.next_after, "1");
        assert!(!first.complete);
        let second = events(&project, &run_id, 1, 10, 1024 * 1024).expect("second page");
        assert_eq!(second.events[0].kind, "custom.future.event");
        assert_eq!(second.next_after, "2");
        assert!(second.complete);
    }

    #[test]
    fn skipped_prefix_bytes_do_not_exhaust_the_return_page() {
        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        let report = initialize_workspace(&project, Some(&sdk)).expect("workspace");
        let mut journal = RunJournal::create(&report.workspace).expect("journal");
        journal
            .record_with_presentation(
                "fixture.large_prefix",
                serde_json::json!({"fixture": true}),
                Some(crate::journal::Presentation::new(
                    crate::journal::PresentationCategory::Info,
                    "x".repeat(1_024),
                )),
            )
            .expect("large prefix");
        journal
            .record("fixture.after_cursor", serde_json::json!({"fixture": true}))
            .expect("selected event");
        let run_id = journal.run_id().to_owned();
        journal.finish(outcome()).expect("summary");

        let page = events(&project, &run_id, 1, 10, 512).expect("page after prefix");
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].kind, "fixture.after_cursor");
        assert_eq!(page.next_after, "2");
        assert!(page.complete);
    }

    #[test]
    fn invalid_run_id_never_reaches_the_filesystem() {
        assert!(matches!(
            validate_run_id("../summary"),
            Err(StudioError::InvalidRunId(_))
        ));
        assert!(matches!(validate_run_id("run-valid-123"), Ok(())));
    }

    #[test]
    fn one_corrupt_run_does_not_hide_the_rest_of_the_workspace() {
        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        initialize_workspace(&project, Some(&sdk)).expect("workspace");
        let run_path = project.join(".tactus/runs/run-9999999999999-1-0");
        fs::create_dir(&run_path).expect("run");
        fs::write(run_path.join("events.jsonl"), b"not-json\n").expect("events");
        fs::write(run_path.join("summary.json"), b"not-json\n").expect("summary");

        let snapshot = inspect(&project, 10).expect("snapshot remains available");
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(snapshot.runs[0].state, "corrupt");
        assert!(matches!(
            snapshot.runs[0].integrity,
            StudioIntegrity::Corrupt
        ));
    }

    fn assert_linked_run_is_not_read(project: &Path, run_id: &str) {
        let error = events(project, run_id, 0, 10, 1024).expect_err("linked run rejected");
        assert!(matches!(error, StudioError::UnsafeTracePath(_)));
        let snapshot = inspect(project, 10).expect("unsafe run remains projectable");
        let encoded = serde_json::to_string(&snapshot).expect("snapshot JSON");
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(snapshot.runs[0].state, "corrupt");
        assert_eq!(snapshot.runs[0].label, "Unsafe run journal");
        assert!(!encoded.contains("DO_NOT_READ_OUTSIDE"));
    }

    fn write_outside_run(path: &Path, run_id: &str) {
        fs::create_dir(path).expect("outside run");
        let event = TraceEvent {
            api: TRACE_API.to_owned(),
            run_id: run_id.to_owned(),
            seq: 1,
            at_unix_ms: 1,
            kind: "runtime.state_transition".to_owned(),
            presentation: Some(crate::journal::Presentation::new(
                crate::journal::PresentationCategory::State,
                "DO_NOT_READ_OUTSIDE",
            )),
            data: serde_json::json!({"state_after":"running"}),
        };
        fs::write(
            path.join("events.jsonl"),
            format!(
                "{}\n",
                serde_json::to_string(&event).expect("outside event JSON")
            ),
        )
        .expect("outside event");
    }

    #[cfg(unix)]
    #[test]
    fn studio_rejects_a_linked_run_directory() {
        use std::os::unix::fs::symlink;

        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        let report = initialize_workspace(&project, Some(&sdk)).expect("workspace");
        let run_id = "run-9999999999999-1-0";
        let outside = temporary.path().join("outside-run");
        write_outside_run(&outside, run_id);
        symlink(&outside, report.workspace.runs_path.join(run_id)).expect("run symlink");
        assert_linked_run_is_not_read(&project, run_id);
    }

    #[cfg(windows)]
    #[test]
    fn studio_rejects_a_run_junction() {
        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        let report = initialize_workspace(&project, Some(&sdk)).expect("workspace");
        let run_id = "run-9999999999999-1-0";
        let outside = temporary.path().join("outside-run");
        write_outside_run(&outside, run_id);
        let linked = report.workspace.runs_path.join(run_id);
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:TACTUS_TEST_JUNCTION -Target $env:TACTUS_TEST_TARGET | Out-Null",
            ])
            .env("TACTUS_TEST_JUNCTION", &linked)
            .env("TACTUS_TEST_TARGET", &outside)
            .output()
            .expect("create junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_linked_run_is_not_read(&project, run_id);
    }

    #[test]
    fn recent_runs_sort_before_applying_the_scan_cap() {
        let temporary = tempdir().expect("temporary");
        let sdk = temporary.path().join("clef-sdk");
        fs::create_dir(&sdk).expect("sdk");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("manifest");
        let project = temporary.path().join("project");
        let report = initialize_workspace(&project, Some(&sdk)).expect("workspace");
        for timestamp in 1..=MAX_RUN_DIRECTORIES_SCANNED + 5 {
            let run_id = format!("run-{timestamp:013}-1-0");
            let run_path = report.workspace.runs_path.join(&run_id);
            fs::create_dir(&run_path).expect("run directory");
            fs::write(run_path.join("events.jsonl"), []).expect("empty events");
        }

        let snapshot = inspect(&project, 1).expect("snapshot");
        assert_eq!(
            snapshot.runs[0].run_id,
            format!("run-{:013}-1-0", MAX_RUN_DIRECTORIES_SCANNED + 5)
        );
    }

    #[test]
    fn newest_run_retention_has_a_fixed_memory_bound() {
        let mut retained = BTreeMap::new();
        for timestamp in 1..=MAX_RUN_DIRECTORIES_SCANNED + 5 {
            let run_id = format!("run-{timestamp:013}-1-0");
            retain_newest_run_directory(&mut retained, run_id.clone(), PathBuf::from(run_id));
            assert!(retained.len() <= MAX_RUN_DIRECTORIES_SCANNED);
        }
        assert_eq!(retained.len(), MAX_RUN_DIRECTORIES_SCANNED);
        assert_eq!(
            retained.keys().next().map(String::as_str),
            Some("run-0000000000006-1-0")
        );
        assert_eq!(
            retained.keys().next_back().map(String::as_str),
            Some("run-0000000002005-1-0")
        );
    }

    #[test]
    fn control_failures_do_not_expose_host_paths() {
        let error = StudioError::BudgetExceeded(PathBuf::from(
            "D:/Users/example/private/.tactus/runs/secret",
        ));
        let encoded = serde_json::to_string(&ControlFailure::new("studio.events", &error))
            .expect("failure json");
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).expect("value")["error"]["code"],
            "read_budget_exceeded"
        );
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains("secret"));
    }
}
