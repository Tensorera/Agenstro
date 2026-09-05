//! Deterministic run-journal queries and conservative maintenance.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use thiserror::Error;

use crate::{
    journal::{RunSummary, TRACE_API, TraceEvent},
    limits::MAX_FRAME_BYTES_CEILING,
    outcome::{classify_outcome, validate_outcome_consistency},
    process::{InvocationKind, ProcessOutcome},
    protocol::TerminalResult,
    workspace::{Workspace, WorkspaceError},
};

/// Version of the `tactus runs` machine-readable result documents.
pub const RUNS_API: &str = "tactus.runs/v1";

const ARCHIVE_DIRECTORY: &str = "archive";
const MAX_SUMMARY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EVENT_LINE_BYTES: usize = MAX_FRAME_BYTES_CEILING + 4 * 1024 * 1024;
const MAX_EVENT_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const MAX_LIST_LIMIT: usize = 10_000;
const MAX_SHOW_EVENT_LIMIT: usize = 1_000;
const MAX_SHOW_EVENT_BYTES: usize = MAX_EVENT_LINE_BYTES;

/// Integrity of the evidence used to project one run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunIntegrity {
    /// Summary and complete event stream agree.
    Ok,
    /// The run is open or a bounded query could not validate the whole trace.
    Partial,
    /// The run directory or trace contradicted the journal contract.
    Corrupt,
}

/// Safe, compact projection of one local run journal.
#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    /// Opaque run identifier.
    pub run_id: String,
    /// Canonical operator state.
    pub state: String,
    /// More specific process outcome, when a valid summary exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_kind: Option<String>,
    /// Stable terminal failure code, when one was safely persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    /// Evidence integrity.
    pub integrity: RunIntegrity,
    /// Journal creation time.
    pub started_unix_ms: u64,
    /// Atomic summary publication time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_unix_ms: Option<u64>,
    /// Latest complete event timestamp observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_unix_ms: Option<u64>,
    /// Complete events observed while validating the journal.
    pub events_recorded: u64,
    /// Whether maintenance must leave this run untouched.
    pub protected: bool,
    /// Stable reasons for protection.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub protection_reasons: Vec<String>,
}

/// Result of `runs list` or `runs unfinished`.
#[derive(Debug, Serialize)]
pub struct RunList {
    /// Result schema.
    pub api: &'static str,
    /// Number matching filters before the result limit.
    pub matched: usize,
    /// Deterministically newest-first records.
    pub runs: Vec<RunRecord>,
}

/// Aggregate run distribution.
#[derive(Debug, Serialize)]
pub struct RunAggregate {
    /// Result schema.
    pub api: &'static str,
    /// Effective lower time bound, if supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_unix_ms: Option<u64>,
    /// Number of matching runs.
    pub matched: usize,
    /// Counts by canonical operator state.
    pub states: BTreeMap<String, u64>,
    /// Counts by specific process outcome.
    pub outcome_kinds: BTreeMap<String, u64>,
    /// Number of records maintenance must preserve.
    pub protected: u64,
    /// Earliest matching start time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earliest_started_unix_ms: Option<u64>,
    /// Latest matching start time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_started_unix_ms: Option<u64>,
}

/// Bounded event page returned by `runs show`.
#[derive(Debug, Serialize)]
pub struct RunEventPage {
    /// Events after the requested cursor.
    pub events: Vec<TraceEvent>,
    /// Last returned sequence, or the input cursor.
    pub next_after: u64,
    /// Whether the reader reached a complete end of file.
    pub complete: bool,
}

/// One run plus a bounded page of its durable events.
#[derive(Debug, Serialize)]
pub struct RunShow {
    /// Result schema.
    pub api: &'static str,
    /// Compact validated run projection.
    pub run: RunRecord,
    /// Bounded event evidence.
    pub page: RunEventPage,
}

/// One planned or completed maintenance action.
#[derive(Clone, Debug, Serialize)]
pub struct MaintenanceAction {
    /// Opaque run identifier.
    pub run_id: String,
    /// Stable action name.
    pub action: String,
}

/// One run deliberately excluded from maintenance.
#[derive(Clone, Debug, Serialize)]
pub struct MaintenanceProtection {
    /// Opaque run identifier.
    pub run_id: String,
    /// Stable protection reason.
    pub reason: String,
}

/// Preview or result of archive/garbage-collection maintenance.
#[derive(Debug, Serialize)]
pub struct MaintenanceReport {
    /// Result schema.
    pub api: &'static str,
    /// `archive` or `gc`.
    pub operation: &'static str,
    /// True unless the caller supplied `--yes`.
    pub dry_run: bool,
    /// Effective upper time bound, when supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_unix_ms: Option<u64>,
    /// Candidate or completed actions.
    pub actions: Vec<MaintenanceAction>,
    /// Old but unsafe or unresolved records preserved.
    pub protected: Vec<MaintenanceProtection>,
}

#[derive(Debug)]
struct ScannedRun {
    path: PathBuf,
    record: RunRecord,
}

#[derive(Debug)]
struct EventFacts {
    count: u64,
    last_event_unix_ms: Option<u64>,
    outcome_unknown: bool,
    complete: bool,
}

/// List active runs with optional state/time filters.
pub fn list(
    start: &Path,
    state: Option<&str>,
    since: Option<&str>,
    limit: usize,
) -> Result<RunList, RunsError> {
    validate_limit(limit)?;
    validate_state_filter(state)?;
    let since_unix_ms = since.map(parse_time_expression).transpose()?;
    let workspace = Workspace::discover(start)?;
    let root = resolve_active_root(&workspace)?;
    let filtered = filtered_runs(scan_root(&root)?, state, since_unix_ms);
    let matched = filtered.len();
    Ok(RunList {
        api: RUNS_API,
        matched,
        runs: filtered
            .into_iter()
            .take(limit)
            .map(|run| run.record)
            .collect(),
    })
}

/// Summarize active runs with optional state/time filters.
pub fn summarize(
    start: &Path,
    state: Option<&str>,
    since: Option<&str>,
) -> Result<RunAggregate, RunsError> {
    validate_state_filter(state)?;
    let since_unix_ms = since.map(parse_time_expression).transpose()?;
    let workspace = Workspace::discover(start)?;
    let root = resolve_active_root(&workspace)?;
    let filtered = filtered_runs(scan_root(&root)?, state, since_unix_ms);
    let mut states = BTreeMap::new();
    let mut outcome_kinds = BTreeMap::new();
    let mut protected = 0_u64;
    let mut earliest = None::<u64>;
    let mut latest = None::<u64>;
    for run in &filtered {
        *states.entry(run.record.state.clone()).or_insert(0) += 1;
        if let Some(kind) = run.record.outcome_kind.as_ref() {
            *outcome_kinds.entry(kind.clone()).or_insert(0) += 1;
        }
        protected += u64::from(run.record.protected);
        earliest = Some(earliest.map_or(run.record.started_unix_ms, |value| {
            value.min(run.record.started_unix_ms)
        }));
        latest = Some(latest.map_or(run.record.started_unix_ms, |value| {
            value.max(run.record.started_unix_ms)
        }));
    }
    Ok(RunAggregate {
        api: RUNS_API,
        since_unix_ms,
        matched: filtered.len(),
        states,
        outcome_kinds,
        protected,
        earliest_started_unix_ms: earliest,
        latest_started_unix_ms: latest,
    })
}

/// List runs without an atomically published terminal summary.
pub fn unfinished(start: &Path, since: Option<&str>, limit: usize) -> Result<RunList, RunsError> {
    list(start, Some("open"), since, limit)
}

/// Show one run and a bounded page of complete durable events.
pub fn show(
    start: &Path,
    run_id: &str,
    after: u64,
    limit: usize,
    max_bytes: usize,
) -> Result<RunShow, RunsError> {
    validate_run_id(run_id)?;
    if limit == 0 || limit > MAX_SHOW_EVENT_LIMIT {
        return Err(RunsError::InvalidArgument(format!(
            "event limit must be between 1 and {MAX_SHOW_EVENT_LIMIT}"
        )));
    }
    if max_bytes == 0 || max_bytes > MAX_SHOW_EVENT_BYTES {
        return Err(RunsError::InvalidArgument(format!(
            "event byte limit must be between 1 and {MAX_SHOW_EVENT_BYTES}"
        )));
    }
    let workspace = Workspace::discover(start)?;
    let root = resolve_active_root(&workspace)?;
    let run = inspect_run(&root, run_id)?;
    let page = read_event_page(&run.path, run_id, after, limit, max_bytes)?;
    Ok(RunShow {
        api: RUNS_API,
        run: run.record,
        page,
    })
}

/// Preview or archive complete old runs below `.tactus/runs/archive`.
pub fn archive(
    start: &Path,
    before: &str,
    confirmed: bool,
) -> Result<MaintenanceReport, RunsError> {
    let before_unix_ms = parse_time_expression(before)?;
    let workspace = Workspace::discover(start)?;
    let active_root = resolve_active_root(&workspace)?;
    let scanned = scan_root(&active_root)?;
    let mut report = maintenance_plan(
        "archive",
        &active_root,
        scanned,
        Some(before_unix_ms),
        confirmed,
    );
    if !confirmed || report.actions.is_empty() {
        return Ok(report);
    }
    let archive_root = resolve_or_create_archive_root(&active_root)?;
    let planned = std::mem::take(&mut report.actions);
    for action in planned {
        let run = match inspect_run(&active_root, &action.run_id) {
            Ok(run) if eligible_for_maintenance(&run.record, Some(before_unix_ms)) => run,
            Ok(run) => {
                report.protected.push(MaintenanceProtection {
                    run_id: action.run_id,
                    reason: protection_reason(&run.record),
                });
                continue;
            }
            Err(_) => {
                report.protected.push(MaintenanceProtection {
                    run_id: action.run_id,
                    reason: "run_changed_or_became_unsafe".to_owned(),
                });
                continue;
            }
        };
        if validate_run_tree_for_mutation(&active_root, &run.path).is_err() {
            report.protected.push(MaintenanceProtection {
                run_id: run.record.run_id,
                reason: "run_changed_or_became_unsafe".to_owned(),
            });
            continue;
        }
        let destination = archive_root.join(&run.record.run_id);
        if fs::symlink_metadata(&destination).is_ok() {
            report.protected.push(MaintenanceProtection {
                run_id: run.record.run_id,
                reason: "archive_destination_exists".to_owned(),
            });
            continue;
        }
        fs::rename(&run.path, &destination).map_err(RunsError::Io)?;
        report.actions.push(MaintenanceAction {
            run_id: run.record.run_id,
            action: "archived".to_owned(),
        });
    }
    Ok(report)
}

/// Preview or delete previously archived, non-ambiguous runs.
pub fn gc(
    start: &Path,
    before: Option<&str>,
    confirmed: bool,
) -> Result<MaintenanceReport, RunsError> {
    let before_unix_ms = before.map(parse_time_expression).transpose()?;
    let workspace = Workspace::discover(start)?;
    let active_root = resolve_active_root(&workspace)?;
    let Some(archive_root) = resolve_archive_root(&active_root)? else {
        return Ok(MaintenanceReport {
            api: RUNS_API,
            operation: "gc",
            dry_run: !confirmed,
            before_unix_ms,
            actions: Vec::new(),
            protected: Vec::new(),
        });
    };
    let scanned = scan_root(&archive_root)?;
    let mut report = maintenance_plan("gc", &archive_root, scanned, before_unix_ms, confirmed);
    if !confirmed || report.actions.is_empty() {
        return Ok(report);
    }
    let planned = std::mem::take(&mut report.actions);
    for action in planned {
        let run = match inspect_run(&archive_root, &action.run_id) {
            Ok(run) if eligible_for_maintenance(&run.record, before_unix_ms) => run,
            Ok(run) => {
                report.protected.push(MaintenanceProtection {
                    run_id: action.run_id,
                    reason: protection_reason(&run.record),
                });
                continue;
            }
            Err(_) => {
                report.protected.push(MaintenanceProtection {
                    run_id: action.run_id,
                    reason: "run_changed_or_became_unsafe".to_owned(),
                });
                continue;
            }
        };
        if validate_run_tree_for_mutation(&archive_root, &run.path).is_err() {
            report.protected.push(MaintenanceProtection {
                run_id: run.record.run_id,
                reason: "run_changed_or_became_unsafe".to_owned(),
            });
            continue;
        }
        fs::remove_dir_all(&run.path).map_err(RunsError::Io)?;
        report.actions.push(MaintenanceAction {
            run_id: run.record.run_id,
            action: "deleted".to_owned(),
        });
    }
    Ok(report)
}

fn filtered_runs(
    runs: Vec<ScannedRun>,
    state: Option<&str>,
    since_unix_ms: Option<u64>,
) -> Vec<ScannedRun> {
    runs.into_iter()
        .filter(|run| {
            since_unix_ms.is_none_or(|since| run.record.started_unix_ms >= since)
                && state.is_none_or(|selected| state_matches(&run.record, selected))
        })
        .collect()
}

fn state_matches(record: &RunRecord, selected: &str) -> bool {
    record.state == selected || record.outcome_kind.as_deref() == Some(selected)
}

fn validate_state_filter(state: Option<&str>) -> Result<(), RunsError> {
    let Some(state) = state else {
        return Ok(());
    };
    if matches!(
        state,
        "succeeded"
            | "failed"
            | "outcome_unknown"
            | "open"
            | "corrupt"
            | "plugin_failed"
            | "process_failed"
            | "protocol_failed"
            | "runtime_failed"
            | "deadline_exceeded"
            | "cancelled"
    ) {
        Ok(())
    } else {
        Err(RunsError::InvalidArgument(format!(
            "unknown run state {state:?}"
        )))
    }
}

fn validate_limit(limit: usize) -> Result<(), RunsError> {
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(RunsError::InvalidArgument(format!(
            "run limit must be between 1 and {MAX_LIST_LIMIT}"
        )));
    }
    Ok(())
}

fn scan_root(root: &Path) -> Result<Vec<ScannedRun>, RunsError> {
    let mut runs = Vec::new();
    for entry in fs::read_dir(root).map_err(RunsError::Io)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let Some(run_id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if validate_run_id(&run_id).is_err() {
            continue;
        }
        match inspect_run(root, &run_id) {
            Ok(run) => runs.push(run),
            Err(_) => runs.push(ScannedRun {
                path: entry.path(),
                record: corrupt_record(&run_id),
            }),
        }
    }
    runs.sort_by(|left, right| {
        right
            .record
            .started_unix_ms
            .cmp(&left.record.started_unix_ms)
            .then_with(|| right.record.run_id.cmp(&left.record.run_id))
    });
    Ok(runs)
}

fn inspect_run(root: &Path, run_id: &str) -> Result<ScannedRun, RunsError> {
    validate_run_id(run_id)?;
    let path = root.join(run_id);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            RunsError::RunNotFound(run_id.to_owned())
        } else {
            RunsError::Io(error)
        }
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(RunsError::UnsafePath);
    }
    let canonical = dunce::canonicalize(&path).map_err(RunsError::Io)?;
    if canonical.parent() != Some(root) {
        return Err(RunsError::UnsafePath);
    }
    let summary = read_summary(&canonical, run_id)?;
    let facts = scan_events(&canonical, run_id, summary.is_some())?;
    let record = match summary {
        None => RunRecord {
            run_id: run_id.to_owned(),
            state: "open".to_owned(),
            outcome_kind: None,
            failure_code: None,
            integrity: RunIntegrity::Partial,
            started_unix_ms: run_id_timestamp(run_id).unwrap_or(0),
            finished_unix_ms: None,
            last_event_unix_ms: facts.last_event_unix_ms,
            events_recorded: facts.count,
            protected: true,
            protection_reasons: vec!["open".to_owned()],
        },
        Some(summary) => {
            if facts.complete && summary.events_recorded != facts.count {
                return Err(RunsError::Corrupt(
                    "summary event count does not match events.jsonl".to_owned(),
                ));
            }
            let mut state = classify_outcome(&summary.outcome).as_str().to_owned();
            let mut reasons = Vec::new();
            if state == "outcome_unknown" || facts.outcome_unknown {
                state = "outcome_unknown".to_owned();
                reasons.push("unresolved_outcome_unknown".to_owned());
            }
            if !facts.complete {
                reasons.push("scan_budget_exceeded".to_owned());
            }
            RunRecord {
                run_id: run_id.to_owned(),
                state,
                outcome_kind: Some(invocation_kind_name(summary.outcome.kind).to_owned()),
                failure_code: terminal_failure_code(&summary.outcome).map(ToOwned::to_owned),
                integrity: if facts.complete {
                    RunIntegrity::Ok
                } else {
                    RunIntegrity::Partial
                },
                started_unix_ms: summary.started_unix_ms,
                finished_unix_ms: Some(summary.finished_unix_ms),
                last_event_unix_ms: facts.last_event_unix_ms,
                events_recorded: facts.count,
                protected: !reasons.is_empty(),
                protection_reasons: reasons,
            }
        }
    };
    Ok(ScannedRun {
        path: canonical,
        record,
    })
}

fn read_summary(path: &Path, run_id: &str) -> Result<Option<RunSummary>, RunsError> {
    let summary_path = path.join("summary.json");
    let metadata = match fs::symlink_metadata(&summary_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(RunsError::Io(error)),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
        || metadata.len() > MAX_SUMMARY_BYTES
    {
        return Err(RunsError::UnsafePath);
    }
    let mut bytes = Vec::new();
    File::open(&summary_path)
        .map_err(RunsError::Io)?
        .take(MAX_SUMMARY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(RunsError::Io)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SUMMARY_BYTES {
        return Err(RunsError::Corrupt(
            "summary exceeds its read budget".to_owned(),
        ));
    }
    let summary: RunSummary = serde_json::from_slice(&bytes).map_err(RunsError::Json)?;
    if summary.api != TRACE_API || summary.run_id != run_id {
        return Err(RunsError::Corrupt(
            "summary identity does not match its run directory".to_owned(),
        ));
    }
    validate_outcome_consistency(&summary.outcome)
        .map_err(|reason| RunsError::Corrupt(reason.to_owned()))?;
    Ok(Some(summary))
}

fn scan_events(path: &Path, run_id: &str, closed: bool) -> Result<EventFacts, RunsError> {
    let event_path = path.join("events.jsonl");
    require_plain_file(&event_path)?;
    let mut reader = BufReader::new(File::open(event_path).map_err(RunsError::Io)?);
    let mut total_bytes = 0_u64;
    let mut expected_seq = 1_u64;
    let mut last_event_unix_ms = None;
    let mut outcome_unknown = false;
    let mut complete = true;
    while let Some(line) = read_bounded_event_line(&mut reader, MAX_EVENT_LINE_BYTES)? {
        let read = line.bytes.len();
        total_bytes = total_bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total_bytes > MAX_EVENT_SCAN_BYTES {
            complete = false;
            break;
        }
        if !line.terminated && !closed {
            complete = false;
            break;
        }
        if !line.terminated {
            return Err(RunsError::Corrupt(
                "closed event stream ends with an incomplete record".to_owned(),
            ));
        }
        let event: TraceEvent =
            serde_json::from_slice(strip_newline(&line.bytes)).map_err(RunsError::Json)?;
        validate_event(&event, run_id, expected_seq)?;
        expected_seq += 1;
        last_event_unix_ms = Some(event.at_unix_ms);
        outcome_unknown |= event_is_outcome_unknown(&event);
    }
    Ok(EventFacts {
        count: expected_seq - 1,
        last_event_unix_ms,
        outcome_unknown,
        complete,
    })
}

fn read_event_page(
    path: &Path,
    run_id: &str,
    after: u64,
    limit: usize,
    max_bytes: usize,
) -> Result<RunEventPage, RunsError> {
    let event_path = path.join("events.jsonl");
    require_plain_file(&event_path)?;
    let mut reader = BufReader::new(File::open(event_path).map_err(RunsError::Io)?);
    let mut expected_seq = 1_u64;
    let mut scanned_bytes = 0_u64;
    let mut retained_bytes = 0_usize;
    let mut events = Vec::new();
    let mut complete = true;
    while let Some(line) = read_bounded_event_line(&mut reader, MAX_EVENT_LINE_BYTES)? {
        let read = line.bytes.len();
        scanned_bytes = scanned_bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if scanned_bytes > MAX_EVENT_SCAN_BYTES {
            return Err(RunsError::ReadBudgetExceeded);
        }
        if !line.terminated {
            complete = false;
            break;
        }
        let event: TraceEvent =
            serde_json::from_slice(strip_newline(&line.bytes)).map_err(RunsError::Json)?;
        validate_event(&event, run_id, expected_seq)?;
        expected_seq += 1;
        if event.seq <= after {
            continue;
        }
        if events.len() >= limit {
            complete = false;
            break;
        }
        if retained_bytes.saturating_add(read) > max_bytes {
            if events.is_empty() {
                return Err(RunsError::InvalidArgument(
                    "the next event exceeds --max-bytes; increase the page byte limit".to_owned(),
                ));
            }
            complete = false;
            break;
        }
        retained_bytes += read;
        events.push(event);
    }
    let next_after = events.last().map_or(after, |event| event.seq);
    Ok(RunEventPage {
        events,
        next_after,
        complete,
    })
}

struct BoundedEventLine {
    bytes: Vec<u8>,
    terminated: bool,
}

fn read_bounded_event_line(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> Result<Option<BoundedEventLine>, RunsError> {
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(RunsError::Io)?;
        if available.is_empty() {
            return Ok((!output.is_empty()).then_some(BoundedEventLine {
                bytes: output,
                terminated: false,
            }));
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if output.len().saturating_add(take) > max_bytes {
            return Err(RunsError::Corrupt(
                "event line exceeds its read budget".to_owned(),
            ));
        }
        let terminated = available.get(take.saturating_sub(1)) == Some(&b'\n');
        output.extend_from_slice(&available[..take]);
        reader.consume(take);
        if terminated {
            return Ok(Some(BoundedEventLine {
                bytes: output,
                terminated: true,
            }));
        }
    }
}

fn validate_event(event: &TraceEvent, run_id: &str, expected_seq: u64) -> Result<(), RunsError> {
    if event.api != TRACE_API || event.run_id != run_id || event.seq != expected_seq {
        return Err(RunsError::Corrupt(
            "event identity or sequence is inconsistent".to_owned(),
        ));
    }
    Ok(())
}

fn event_is_outcome_unknown(event: &TraceEvent) -> bool {
    event.kind == "runtime.outcome_unknown"
        || (event.kind == "runtime.state_transition"
            && event
                .data
                .get("state_after")
                .and_then(serde_json::Value::as_str)
                == Some("outcome_unknown"))
}

fn maintenance_plan(
    operation: &'static str,
    root: &Path,
    runs: Vec<ScannedRun>,
    before_unix_ms: Option<u64>,
    confirmed: bool,
) -> MaintenanceReport {
    let mut actions = Vec::new();
    let mut protected = Vec::new();
    for run in runs {
        let comparison_time = run
            .record
            .finished_unix_ms
            .unwrap_or(run.record.started_unix_ms);
        if before_unix_ms.is_some_and(|before| comparison_time >= before) {
            continue;
        }
        if eligible_for_maintenance(&run.record, before_unix_ms) {
            if validate_run_tree_for_mutation(root, &run.path).is_err() {
                protected.push(MaintenanceProtection {
                    run_id: run.record.run_id,
                    reason: "run_changed_or_became_unsafe".to_owned(),
                });
                continue;
            }
            actions.push(MaintenanceAction {
                run_id: run.record.run_id,
                action: match (operation, confirmed) {
                    ("archive", false) => "would_archive",
                    ("archive", true) => "archive",
                    ("gc", false) => "would_delete",
                    ("gc", true) => "delete",
                    _ => "unknown",
                }
                .to_owned(),
            });
        } else {
            let reason = protection_reason(&run.record);
            protected.push(MaintenanceProtection {
                run_id: run.record.run_id,
                reason,
            });
        }
    }
    MaintenanceReport {
        api: RUNS_API,
        operation,
        dry_run: !confirmed,
        before_unix_ms,
        actions,
        protected,
    }
}

fn eligible_for_maintenance(record: &RunRecord, before_unix_ms: Option<u64>) -> bool {
    record.integrity == RunIntegrity::Ok
        && !record.protected
        && record.finished_unix_ms.is_some()
        && before_unix_ms.is_none_or(|before| {
            record
                .finished_unix_ms
                .is_some_and(|finished| finished < before)
        })
}

fn protection_reason(record: &RunRecord) -> String {
    record
        .protection_reasons
        .first()
        .cloned()
        .unwrap_or_else(|| match record.integrity {
            RunIntegrity::Ok => "run_changed_or_no_longer_eligible".to_owned(),
            RunIntegrity::Partial => "open".to_owned(),
            RunIntegrity::Corrupt => "corrupt".to_owned(),
        })
}

fn validate_run_tree_for_mutation(root: &Path, path: &Path) -> Result<(), RunsError> {
    let canonical = dunce::canonicalize(path).map_err(RunsError::Io)?;
    if canonical.parent() != Some(root) {
        return Err(RunsError::UnsafePath);
    }
    let metadata = fs::symlink_metadata(path).map_err(RunsError::Io)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(RunsError::UnsafePath);
    }
    let mut seen_events = false;
    let mut seen_summary = false;
    for entry in fs::read_dir(path).map_err(RunsError::Io)? {
        let entry = entry.map_err(RunsError::Io)?;
        let name = entry.file_name();
        let metadata = fs::symlink_metadata(entry.path()).map_err(RunsError::Io)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&metadata)
        {
            return Err(RunsError::UnsafePath);
        }
        match name.to_str() {
            Some("events.jsonl") => seen_events = true,
            Some("summary.json") => seen_summary = true,
            _ => return Err(RunsError::UnsafePath),
        }
    }
    if !seen_events || !seen_summary {
        return Err(RunsError::Corrupt(
            "complete run lacks events.jsonl or summary.json".to_owned(),
        ));
    }
    Ok(())
}

fn resolve_active_root(workspace: &Workspace) -> Result<PathBuf, RunsError> {
    let control = dunce::canonicalize(&workspace.control).map_err(RunsError::Io)?;
    resolve_plain_directory(&workspace.runs_path, &control)
}

fn resolve_archive_root(active_root: &Path) -> Result<Option<PathBuf>, RunsError> {
    let archive = active_root.join(ARCHIVE_DIRECTORY);
    match fs::symlink_metadata(&archive) {
        Ok(_) => resolve_plain_directory(&archive, active_root).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RunsError::Io(error)),
    }
}

fn resolve_or_create_archive_root(active_root: &Path) -> Result<PathBuf, RunsError> {
    if let Some(root) = resolve_archive_root(active_root)? {
        return Ok(root);
    }
    let archive = active_root.join(ARCHIVE_DIRECTORY);
    fs::create_dir(&archive).map_err(RunsError::Io)?;
    resolve_plain_directory(&archive, active_root)
}

fn resolve_plain_directory(path: &Path, expected_parent: &Path) -> Result<PathBuf, RunsError> {
    let metadata = fs::symlink_metadata(path).map_err(RunsError::Io)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(RunsError::UnsafePath);
    }
    let canonical = dunce::canonicalize(path).map_err(RunsError::Io)?;
    if canonical.parent() != Some(expected_parent) {
        return Err(RunsError::UnsafePath);
    }
    Ok(canonical)
}

fn require_plain_file(path: &Path) -> Result<(), RunsError> {
    let metadata = fs::symlink_metadata(path).map_err(RunsError::Io)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(RunsError::UnsafePath);
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn parse_time_expression(value: &str) -> Result<u64, RunsError> {
    parse_time_expression_at(value, unix_millis())
}

fn parse_time_expression_at(value: &str, now_unix_ms: u64) -> Result<u64, RunsError> {
    if value == "now" {
        return Ok(now_unix_ms);
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value
            .parse()
            .map_err(|_| RunsError::InvalidArgument("time value is outside u64".to_owned()));
    }
    let (number, multiplier) = [
        ("ms", 1_u64),
        ("s", 1_000),
        ("m", 60_000),
        ("h", 3_600_000),
        ("d", 86_400_000),
        ("w", 604_800_000),
    ]
    .into_iter()
    .find_map(|(suffix, multiplier)| {
        value
            .strip_suffix(suffix)
            .map(|number| (number, multiplier))
    })
    .ok_or_else(|| {
        RunsError::InvalidArgument(
            "time must be Unix milliseconds, `now`, or a relative value such as 24h or 7d"
                .to_owned(),
        )
    })?;
    let amount: u64 = number.parse().map_err(|_| {
        RunsError::InvalidArgument("relative time must start with an unsigned integer".to_owned())
    })?;
    let duration = amount.checked_mul(multiplier).ok_or_else(|| {
        RunsError::InvalidArgument("relative time is outside the supported range".to_owned())
    })?;
    now_unix_ms.checked_sub(duration).ok_or_else(|| {
        RunsError::InvalidArgument("relative time predates the Unix epoch".to_owned())
    })
}

fn terminal_failure_code(outcome: &ProcessOutcome) -> Option<&str> {
    match outcome.terminal.as_ref() {
        Some(TerminalResult::Failure { error }) => Some(&error.code),
        _ => None,
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

fn corrupt_record(run_id: &str) -> RunRecord {
    RunRecord {
        run_id: run_id.to_owned(),
        state: "corrupt".to_owned(),
        outcome_kind: None,
        failure_code: None,
        integrity: RunIntegrity::Corrupt,
        started_unix_ms: run_id_timestamp(run_id).unwrap_or(0),
        finished_unix_ms: None,
        last_event_unix_ms: None,
        events_recorded: 0,
        protected: true,
        protection_reasons: vec!["corrupt".to_owned()],
    }
}

fn validate_run_id(run_id: &str) -> Result<(), RunsError> {
    let valid = run_id.starts_with("run-")
        && run_id.len() <= 128
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(RunsError::InvalidRunId)
    }
}

fn run_id_timestamp(run_id: &str) -> Option<u64> {
    run_id.strip_prefix("run-")?.split('-').next()?.parse().ok()
}

fn strip_newline(bytes: &[u8]) -> &[u8] {
    let without_lf = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Failure while querying or maintaining local run journals.
#[derive(Debug, Error)]
pub enum RunsError {
    /// Workspace discovery failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Filesystem operation failed.
    #[error("run-journal filesystem operation failed: {0}")]
    Io(#[source] io::Error),
    /// JSON evidence was malformed.
    #[error("run-journal JSON is malformed: {0}")]
    Json(#[from] serde_json::Error),
    /// A command option was invalid.
    #[error("invalid runs argument: {0}")]
    InvalidArgument(String),
    /// A requested run does not exist.
    #[error("run was not found: {0}")]
    RunNotFound(String),
    /// A requested identifier was not path-safe.
    #[error("invalid run id")]
    InvalidRunId,
    /// A link, reparse point, or escaped path was encountered.
    #[error("run journal path is not a plain contained path")]
    UnsafePath,
    /// Durable evidence contradicted its schema or sequence.
    #[error("run journal is corrupt: {0}")]
    Corrupt(String),
    /// A bounded query would need to scan an unsafe amount of evidence.
    #[error("run journal read budget exceeded")]
    ReadBudgetExceeded,
}

impl RunsError {
    /// Stable code for JSON clients.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Workspace(_) => "runs_workspace_invalid",
            Self::Io(_) => "runs_io_failed",
            Self::Json(_) | Self::Corrupt(_) => "run_corrupt",
            Self::ReadBudgetExceeded => "runs_read_budget_exceeded",
            Self::InvalidArgument(_) => "runs_argument_invalid",
            Self::RunNotFound(_) => "run_not_found",
            Self::InvalidRunId => "run_id_invalid",
            Self::UnsafePath => "run_path_unsafe",
        }
    }

    /// Bounded public diagnostic that does not expose host paths.
    #[must_use]
    pub fn public_message(&self) -> String {
        match self {
            Self::InvalidArgument(message) | Self::Corrupt(message) => message.clone(),
            Self::RunNotFound(run_id) => format!("run {run_id:?} was not found"),
            Self::InvalidRunId => "the run id is invalid".to_owned(),
            Self::UnsafePath => "the run path is not a plain contained directory".to_owned(),
            Self::Workspace(_) => "the Tactus workspace is unavailable or invalid".to_owned(),
            Self::Io(_) => "the run journal could not be read or changed".to_owned(),
            Self::Json(_) => "the run journal contains malformed JSON".to_owned(),
            Self::ReadBudgetExceeded => {
                "the run journal exceeds the bounded query scan budget".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{
        journal::RunJournal,
        process::ProcessOutcome,
        protocol::{PluginFailure, TerminalResult},
    };
    use tempfile::tempdir;

    fn workspace_fixture() -> (tempfile::TempDir, Workspace) {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        fs::create_dir_all(&workspace.scripts_path).expect("scripts");
        fs::create_dir_all(&workspace.runs_path).expect("runs");
        fs::write(&workspace.config_path, "placeholder").expect("config");
        fs::write(&workspace.prompt_path, "placeholder").expect("prompt");
        (temporary, workspace)
    }

    fn outcome(kind: InvocationKind, failure_code: Option<&str>) -> ProcessOutcome {
        let terminal = if kind == InvocationKind::Succeeded {
            Some(TerminalResult::Success {
                value: serde_json::json!({"fixture": true}),
            })
        } else {
            failure_code.map(|code| TerminalResult::Failure {
                error: PluginFailure {
                    code: code.to_owned(),
                    message: "bounded fixture".to_owned(),
                    details: None,
                },
            })
        };
        ProcessOutcome {
            kind,
            exit_code: Some(if kind == InvocationKind::Succeeded {
                0
            } else {
                1
            }),
            terminal,
            frames_seen: u64::from(kind == InvocationKind::Succeeded || failure_code.is_some()),
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms: 1,
            progress: None,
        }
    }

    fn completed_run(workspace: &Workspace, selected: ProcessOutcome) -> String {
        let mut journal = RunJournal::create(workspace).expect("journal");
        journal
            .record("fixture.event", serde_json::json!({"fixture":true}))
            .expect("event");
        let run_id = journal.run_id().to_owned();
        journal.finish(selected).expect("summary");
        run_id
    }

    #[test]
    fn relative_time_filters_are_stable() {
        let now = 10 * 86_400_000;
        assert_eq!(
            parse_time_expression_at("24h", now).expect("24h"),
            now - 86_400_000
        );
        assert_eq!(
            parse_time_expression_at("7d", now).expect("7d"),
            now - 7 * 86_400_000
        );
        assert_eq!(parse_time_expression_at("123", now).expect("absolute"), 123);
        assert!(parse_time_expression_at("yesterday", now).is_err());
    }

    #[test]
    fn state_filter_accepts_canonical_and_specific_states() {
        for state in [
            "succeeded",
            "failed",
            "outcome_unknown",
            "open",
            "corrupt",
            "protocol_failed",
        ] {
            validate_state_filter(Some(state)).expect("supported state");
        }
        assert!(validate_state_filter(Some("maybe")).is_err());
    }

    #[test]
    fn list_classifies_terminal_unknown_and_show_reads_events() {
        let (_temporary, workspace) = workspace_fixture();
        let run_id = completed_run(
            &workspace,
            outcome(InvocationKind::PluginFailed, Some("outcome_unknown")),
        );
        let listed = list(&workspace.root, Some("outcome_unknown"), None, 10).expect("list");
        assert_eq!(listed.matched, 1);
        assert!(listed.runs[0].protected);
        let shown = show(&workspace.root, &run_id, 0, 10, 1024 * 1024).expect("show");
        assert_eq!(shown.page.events.len(), 1);
        assert!(shown.page.complete);
    }

    #[test]
    fn a_page_that_cannot_fit_its_next_event_errors_instead_of_stalling() {
        let (_temporary, workspace) = workspace_fixture();
        let run_id = completed_run(&workspace, outcome(InvocationKind::Succeeded, None));
        let error = show(&workspace.root, &run_id, 0, 10, 1).expect_err("page error");
        assert!(matches!(error, RunsError::InvalidArgument(_)));
    }

    #[test]
    fn bounded_event_reader_rejects_an_oversized_unterminated_record() {
        let mut input = Cursor::new(vec![b'x'; 65]);
        assert!(matches!(
            read_bounded_event_line(&mut input, 32),
            Err(RunsError::Corrupt(_))
        ));
    }

    #[test]
    fn contradictory_summary_is_classified_as_corrupt_and_protected() {
        let (_temporary, workspace) = workspace_fixture();
        let run_id = completed_run(&workspace, outcome(InvocationKind::Succeeded, None));
        let summary_path = workspace.runs_path.join(&run_id).join("summary.json");
        let mut summary: RunSummary =
            serde_json::from_slice(&fs::read(&summary_path).expect("summary bytes"))
                .expect("summary");
        summary.outcome.exit_code = Some(1);
        fs::write(
            &summary_path,
            serde_json::to_vec(&summary).expect("encoded summary"),
        )
        .expect("rewrite summary");

        let listed = list(&workspace.root, None, None, 10).expect("list");
        assert_eq!(listed.runs.len(), 1);
        assert_eq!(listed.runs[0].state, "corrupt");
        assert!(listed.runs[0].protected);
    }

    #[test]
    fn archive_and_gc_are_dry_run_by_default_and_preserve_unsafe_states() {
        let (_temporary, workspace) = workspace_fixture();
        let succeeded = completed_run(&workspace, outcome(InvocationKind::Succeeded, None));
        let unsafe_tree = completed_run(&workspace, outcome(InvocationKind::Succeeded, None));
        fs::write(
            workspace
                .runs_path
                .join(&unsafe_tree)
                .join("unexpected.txt"),
            "not part of the journal contract",
        )
        .expect("unexpected file");
        let unknown = completed_run(&workspace, outcome(InvocationKind::ProtocolFailed, None));
        let open = {
            let journal = RunJournal::create(&workspace).expect("open journal");
            journal.run_id().to_owned()
        };
        let corrupt = "run-1-corrupt-0";
        fs::create_dir(workspace.runs_path.join(corrupt)).expect("corrupt run");

        let preview = archive(&workspace.root, &u64::MAX.to_string(), false).expect("preview");
        assert!(preview.dry_run);
        assert!(
            preview
                .actions
                .iter()
                .any(|action| action.run_id == succeeded)
        );
        for protected in [&unknown, &open, corrupt] {
            assert!(
                preview
                    .protected
                    .iter()
                    .any(|entry| entry.run_id == *protected),
                "missing protection for {protected}"
            );
        }
        assert!(preview.protected.iter().any(|entry| {
            entry.run_id == unsafe_tree && entry.reason == "run_changed_or_became_unsafe"
        }));
        assert!(workspace.runs_path.join(&succeeded).is_dir());

        let applied = archive(&workspace.root, &u64::MAX.to_string(), true).expect("archive");
        assert!(!applied.dry_run);
        let archived = workspace.runs_path.join(ARCHIVE_DIRECTORY).join(&succeeded);
        assert!(archived.is_dir());
        assert!(workspace.runs_path.join(&unknown).is_dir());

        let gc_preview = gc(&workspace.root, None, false).expect("gc preview");
        assert!(gc_preview.dry_run);
        assert!(archived.is_dir());
        let gc_applied = gc(&workspace.root, None, true).expect("gc");
        assert!(!gc_applied.dry_run);
        assert!(!archived.exists());
    }

    #[test]
    fn newest_selection_is_deterministic_beyond_two_thousand_directories() {
        let (_temporary, workspace) = workspace_fixture();
        for timestamp in 1..=2_005_u64 {
            fs::create_dir(workspace.runs_path.join(format!("run-{timestamp}-1-0")))
                .expect("run directory");
        }
        let listed = list(&workspace.root, None, None, 1).expect("list");
        assert_eq!(listed.matched, 2_005);
        assert_eq!(listed.runs[0].run_id, "run-2005-1-0");
    }
}
