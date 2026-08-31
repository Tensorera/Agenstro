//! Durable, bounded elicitation-session storage and answer control.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::workspace::{Workspace, WorkspaceError};

/// Version of the durable session document and its control projections.
pub const SESSION_API: &str = "agenstro.session/v1";

const MAX_LIST_LIMIT: usize = 200;
const MAX_SESSION_ENTRIES_SCANNED: usize = 2_000;
const MAX_SESSION_DOCUMENT_BYTES: u64 = 1024 * 1024;
// Motivo captures at most 9 MiB from a control process. Keep both the source
// documents and their serialized data projection below 8 MiB so the control
// envelope and JSON framing retain almost 1 MiB of headroom.
const MAX_SESSION_LIST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TRANSCRIPT_RECORD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ANSWERED_AXES: usize = 256;
const MAX_FINDINGS: usize = 12;
const MAX_OPTIONS: usize = 6;
const MAX_COORDINATES: usize = 12;
const MAX_REMAINING_AXES: usize = 64;
const MAX_NOTE_BYTES: usize = 4_096;
const SESSION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// One validated session listing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionList {
    /// Projection API.
    pub api: &'static str,
    /// Most recently updated sessions first.
    pub sessions: Vec<SessionView>,
}

#[derive(Debug)]
struct RankedSession {
    updated_unix_ms: u64,
    session: SessionView,
}

impl PartialEq for RankedSession {
    fn eq(&self, other: &Self) -> bool {
        self.updated_unix_ms == other.updated_unix_ms
            && self.session.session_id == other.session.session_id
    }
}

impl Eq for RankedSession {}

impl Ord for RankedSession {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap keeps the greatest item at its root. Reverse the desired
        // list order here so the least recent retained session is replaced
        // whenever a better candidate arrives.
        other
            .updated_unix_ms
            .cmp(&self.updated_unix_ms)
            .then_with(|| self.session.session_id.cmp(&other.session.session_id))
    }
}

impl PartialOrd for RankedSession {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Durable session state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Ready for a planner step.
    Planning,
    /// A person must answer the pending brief.
    AwaitingAnswer,
    /// The planner delivered the artifact.
    Delivered,
    /// The session was explicitly abandoned.
    Abandoned,
}

/// One durable session document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    /// Session API.
    pub api: String,
    /// Opaque, path-safe session identifier.
    pub session_id: String,
    /// Human-readable label.
    pub label: String,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Delivered-brief counter and answer CAS token.
    pub turn: String,
    /// Present exactly while awaiting an answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<SessionBrief>,
    /// Right-biased answers, unique by axis.
    pub answered: Vec<AnsweredAxis>,
    /// Creation time as JavaScript-safe decimal text.
    pub started_unix_ms: String,
    /// Last update time as JavaScript-safe decimal text.
    pub updated_unix_ms: String,
    /// Additive v1 fields retained across atomic rewrites.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// The one brief currently shown to a person.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBrief {
    /// Session API.
    pub api: String,
    /// Parent session identifier.
    pub session_id: String,
    /// Parent turn token.
    pub turn: String,
    /// New knowledge that makes the question answerable.
    pub findings: Vec<SessionFinding>,
    /// Exactly one decision.
    pub question: SessionQuestion,
    /// Consequences keyed by option identifier.
    pub stakes: Vec<SessionConsequence>,
    /// Optional planner-authored unattended default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_option: Option<String>,
    /// Axes that may still be asked.
    pub remaining_surface: Vec<String>,
    /// Axes that must still be asked.
    pub remaining_floor: Vec<String>,
    /// Additive v1 fields retained while the brief is pending or transcribed.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// One bounded finding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFinding {
    /// Short factual summary.
    pub summary: String,
    /// Optional detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Optional evidence source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Additive v1 fields.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// One typed question.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQuestion {
    /// Stable decision axis.
    pub axis: String,
    /// Human-readable prompt.
    pub prompt: String,
    /// Two to six choices.
    pub options: Vec<SessionOption>,
    /// Cost of changing this decision later.
    pub reversibility: Reversibility,
    /// Stable axes on which this question depends.
    pub depends_on: Vec<String>,
    /// Additive v1 fields.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// One labelled option.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOption {
    /// Stable option identifier within the question.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Open comparison coordinates.
    pub coordinates: BTreeMap<String, String>,
    /// Optional rationale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// Additive v1 fields.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// One option consequence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConsequence {
    /// Referenced option identifier.
    pub option: String,
    /// Human-readable effect.
    pub effect: String,
    /// Cost of reversing the effect.
    pub reversibility: Reversibility,
    /// Additive v1 fields.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// Closed reversibility vocabulary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    /// Cheap to reverse.
    Reversible,
    /// Possible but costly to reverse.
    Costly,
    /// Must be treated as irreversible.
    Irreversible,
}

/// Latest answer for one stable axis.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnsweredAxis {
    /// Stable axis identity.
    pub axis: String,
    /// Chosen option identity.
    pub option: String,
    /// Label shown when the choice was made.
    pub label: String,
    /// Whether a planner-authored default supplied the choice.
    pub defaulted: bool,
    /// Answer time as decimal text.
    pub answered_at_unix_ms: String,
    /// Additive v1 fields retained for historical answers.
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// Stable control failure envelope for session commands.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlFailure {
    /// Control API.
    pub api: &'static str,
    /// Stable command name.
    pub command: &'static str,
    /// Always `error`.
    pub status: &'static str,
    /// Public, path-redacted failure.
    pub error: SessionFailure,
}

/// Public session failure.
#[derive(Debug, Serialize)]
pub struct SessionFailure {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Bounded path-free diagnostic.
    pub message: &'static str,
}

impl SessionControlFailure {
    /// Project an internal failure into `tactus.control/v1`.
    #[must_use]
    pub fn new(command: &'static str, error: &SessionError) -> Self {
        Self {
            api: crate::studio::CONTROL_API,
            command,
            status: "error",
            error: SessionFailure {
                code: error.code(),
                message: error.public_message(),
            },
        }
    }
}

/// List validated sessions with newest updates first.
pub fn list(start: &Path, limit: usize) -> Result<SessionList, SessionError> {
    if !(1..=MAX_LIST_LIMIT).contains(&limit) {
        return Err(SessionError::InvalidArgument);
    }
    let workspace = Workspace::discover(start).map_err(SessionError::Workspace)?;
    let Some(root) = resolve_sessions_root(&workspace, true)? else {
        return Ok(SessionList {
            api: SESSION_API,
            sessions: Vec::new(),
        });
    };
    let mut sessions = BinaryHeap::with_capacity(limit);
    let mut bytes_read = 0u64;
    for (index, entry) in fs::read_dir(&root).map_err(SessionError::Io)?.enumerate() {
        if index >= MAX_SESSION_ENTRIES_SCANNED {
            return Err(SessionError::Corrupt("too many session entries".to_owned()));
        }
        let entry = entry.map_err(SessionError::Io)?;
        let Some(identifier) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_session_id(&identifier) {
            continue;
        }
        let resolved = resolve_listed_session_directory(&root, &entry)?;
        let (session, document_bytes) = read_session_counted(&resolved, &identifier)?;
        bytes_read = bytes_read.checked_add(document_bytes).ok_or_else(|| {
            SessionError::Corrupt("session list byte budget overflowed".to_owned())
        })?;
        if bytes_read > MAX_SESSION_LIST_BYTES {
            return Err(SessionError::Corrupt(
                "session list exceeds the 8 MiB read budget".to_owned(),
            ));
        }
        let ranked = RankedSession {
            updated_unix_ms: decimal_u64(&session.updated_unix_ms),
            session,
        };
        if sessions.len() < limit {
            sessions.push(ranked);
        } else if sessions
            .peek()
            .is_some_and(|least_recent| ranked < *least_recent)
        {
            sessions.pop();
            sessions.push(ranked);
        }
    }
    let mut sessions = sessions
        .into_iter()
        .map(|ranked| ranked.session)
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        decimal_u64(&right.updated_unix_ms)
            .cmp(&decimal_u64(&left.updated_unix_ms))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let list = SessionList {
        api: SESSION_API,
        sessions,
    };
    ensure_list_projection_budget(&list)?;
    Ok(list)
}

/// Show one validated session.
pub fn show(start: &Path, session_id: &str) -> Result<SessionView, SessionError> {
    let (_, directory) = locate_session(start, session_id)?;
    read_session(&directory, session_id)
}

/// Apply one human answer under a cross-process turn-CAS lock.
pub fn answer(
    start: &Path,
    session_id: &str,
    turn: &str,
    axis: &str,
    option: &str,
    note: Option<&str>,
) -> Result<SessionView, SessionError> {
    if require_decimal(turn, "answer turn").is_err() {
        return Err(SessionError::InvalidArgument);
    }
    if !is_axis_id(axis) {
        return Err(SessionError::AxisMismatch);
    }
    if !is_option_id(option) {
        return Err(SessionError::OptionInvalid);
    }
    if note.is_some_and(|value| value.len() > MAX_NOTE_BYTES || value.contains('\0')) {
        return Err(SessionError::InvalidArgument);
    }
    let (_, directory) = locate_session(start, session_id)?;
    let _lock = lock_session(&directory)?;
    let mut session = read_session(&directory, session_id)?;
    if session.turn != turn {
        return Err(SessionError::TurnStale);
    }
    if session.state != SessionState::AwaitingAnswer {
        return Err(SessionError::StateInvalid);
    }
    let pending = session.pending.take().ok_or(SessionError::StateInvalid)?;
    if pending.question.axis != axis {
        return Err(SessionError::AxisMismatch);
    }
    let selected = pending
        .question
        .options
        .iter()
        .find(|candidate| candidate.id == option)
        .ok_or(SessionError::OptionInvalid)?;
    let replacing_axis = session.answered.iter().any(|prior| prior.axis == axis);
    if !replacing_axis && session.answered.len() >= MAX_ANSWERED_AXES {
        return Err(SessionError::StateInvalid);
    }
    let previous_updated = require_decimal(&session.updated_unix_ms, "session update time")?;
    let answered_at = unix_ms()?.max(previous_updated).to_string();
    let answered = AnsweredAxis {
        axis: axis.to_owned(),
        option: option.to_owned(),
        label: selected.label.clone(),
        defaulted: false,
        answered_at_unix_ms: answered_at.clone(),
        extensions: BTreeMap::new(),
    };
    project_answer(&mut session, &answered, previous_updated)?;
    validate_session(&session, session_id)?;

    // The transcript is appended first so a returned success never loses the
    // submitted note. If atomic publication then fails, a retry trims any
    // partial tail and reuses the complete record with the deterministic
    // answerId instead of appending it twice.
    let recorded = append_transcript(&directory, &pending, &answered, note)?;
    if recorded != answered {
        project_answer(&mut session, &recorded, previous_updated)?;
        validate_session(&session, session_id)?;
    }
    replace_session(&directory, &session)?;
    Ok(session)
}

fn project_answer(
    session: &mut SessionView,
    answer: &AnsweredAxis,
    previous_updated: u64,
) -> Result<(), SessionError> {
    session.answered.retain(|prior| prior.axis != answer.axis);
    session.answered.push(answer.clone());
    session.pending = None;
    session.state = SessionState::Planning;
    session.updated_unix_ms = previous_updated
        .max(require_decimal(&answer.answered_at_unix_ms, "answer time")?)
        .to_string();
    Ok(())
}

fn locate_session(start: &Path, session_id: &str) -> Result<(Workspace, PathBuf), SessionError> {
    if !is_session_id(session_id) {
        return Err(SessionError::InvalidId);
    }
    let workspace = Workspace::discover(start).map_err(SessionError::Workspace)?;
    let root = resolve_sessions_root(&workspace, false)?.ok_or(SessionError::NotFound)?;
    let directory = resolve_session_directory(&root, session_id)?;
    Ok((workspace, directory))
}

fn resolve_sessions_root(
    workspace: &Workspace,
    missing_ok: bool,
) -> Result<Option<PathBuf>, SessionError> {
    let metadata = match fs::symlink_metadata(&workspace.sessions_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound && missing_ok => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(SessionError::NotFound);
        }
        Err(error) => return Err(SessionError::Io(error)),
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || directory_is_reparse_point(&metadata)
    {
        return Err(SessionError::Corrupt(
            "sessions path is not a plain directory".to_owned(),
        ));
    }
    let root = dunce::canonicalize(&workspace.sessions_path).map_err(SessionError::Io)?;
    let control = dunce::canonicalize(&workspace.control).map_err(SessionError::Io)?;
    let workspace_root = dunce::canonicalize(&workspace.root).map_err(SessionError::Io)?;
    if root.parent() != Some(control.as_path()) || !control.starts_with(&workspace_root) {
        return Err(SessionError::Corrupt(
            "sessions path escaped the workspace".to_owned(),
        ));
    }
    Ok(Some(root))
}

fn resolve_session_directory(root: &Path, session_id: &str) -> Result<PathBuf, SessionError> {
    if !has_exact_child_name(root, session_id)? {
        return Err(SessionError::NotFound);
    }
    resolve_session_path(root, root.join(session_id))
}

fn resolve_listed_session_directory(
    root: &Path,
    entry: &fs::DirEntry,
) -> Result<PathBuf, SessionError> {
    // The entry came from this exact read_dir pass, so its OsString already
    // proves exact child-name identity without rescanning the parent.
    resolve_session_path(root, entry.path())
}

fn resolve_session_path(root: &Path, path: PathBuf) -> Result<PathBuf, SessionError> {
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(SessionError::NotFound);
        }
        Err(error) => return Err(SessionError::Io(error)),
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || directory_is_reparse_point(&metadata)
    {
        return Err(SessionError::Corrupt(
            "session path is not a plain directory".to_owned(),
        ));
    }
    let resolved = dunce::canonicalize(path).map_err(SessionError::Io)?;
    if resolved.parent() != Some(root) {
        return Err(SessionError::Corrupt(
            "session path escaped the sessions directory".to_owned(),
        ));
    }
    Ok(resolved)
}

fn has_exact_child_name(root: &Path, expected: &str) -> Result<bool, SessionError> {
    for (index, entry) in fs::read_dir(root).map_err(SessionError::Io)?.enumerate() {
        if index >= MAX_SESSION_ENTRIES_SCANNED {
            return Err(SessionError::Corrupt("too many session entries".to_owned()));
        }
        if entry.map_err(SessionError::Io)?.file_name() == expected {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_session(directory: &Path, expected_id: &str) -> Result<SessionView, SessionError> {
    read_session_counted(directory, expected_id).map(|(session, _)| session)
}

fn read_session_counted(
    directory: &Path,
    expected_id: &str,
) -> Result<(SessionView, u64), SessionError> {
    let path = directory.join("session.json");
    let file = open_existing_plain(&path, directory, "session document is missing")?;
    let document_bytes = file.metadata().map_err(SessionError::Io)?.len();
    let mut bytes = Vec::with_capacity(
        usize::try_from(document_bytes.min(MAX_SESSION_DOCUMENT_BYTES)).unwrap_or(0),
    );
    file.take(MAX_SESSION_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(SessionError::Io)?;
    if bytes.len() as u64 > MAX_SESSION_DOCUMENT_BYTES {
        return Err(SessionError::Corrupt(
            "session document exceeds 1 MiB".to_owned(),
        ));
    }
    let value = crate::protocol::decode_json(&bytes)
        .map_err(|error| SessionError::Corrupt(format!("invalid session JSON: {error}")))?;
    let session: SessionView = serde_json::from_value(value)
        .map_err(|error| SessionError::Corrupt(format!("invalid session shape: {error}")))?;
    validate_session(&session, expected_id)?;
    Ok((session, bytes.len() as u64))
}

fn validate_session(session: &SessionView, expected_id: &str) -> Result<(), SessionError> {
    corrupt_if(session.api != SESSION_API, "unknown session API")?;
    corrupt_if(
        !is_session_id(&session.session_id),
        "invalid stored session id",
    )?;
    corrupt_if(
        session.session_id != expected_id,
        "session id does not match its directory",
    )?;
    validate_text(&session.label, 256, true, "session label")?;
    let turn = require_decimal(&session.turn, "session turn")?;
    let started = require_decimal(&session.started_unix_ms, "session start time")?;
    let updated = require_decimal(&session.updated_unix_ms, "session update time")?;
    corrupt_if(updated < started, "session update precedes its start")?;
    corrupt_if(
        (session.state == SessionState::AwaitingAnswer) != session.pending.is_some(),
        "pending is present exactly while awaiting an answer",
    )?;
    if let Some(brief) = &session.pending {
        validate_brief(brief)?;
        corrupt_if(
            brief.session_id != session.session_id,
            "brief session id mismatch",
        )?;
        corrupt_if(
            require_decimal(&brief.turn, "brief turn")? != turn,
            "brief turn mismatch",
        )?;
    }
    corrupt_if(
        session.answered.len() > MAX_ANSWERED_AXES,
        "too many answered axes",
    )?;
    let mut axes = BTreeSet::new();
    for answer in &session.answered {
        corrupt_if(!is_axis_id(&answer.axis), "invalid answered axis")?;
        corrupt_if(
            !axes.insert(answer.axis.as_str()),
            "duplicate answered axis",
        )?;
        corrupt_if(!is_option_id(&answer.option), "invalid answered option")?;
        validate_text(&answer.label, 160, true, "answer label")?;
        let answered_at = require_decimal(&answer.answered_at_unix_ms, "answer time")?;
        corrupt_if(
            answered_at < started || answered_at > updated,
            "answer time is out of range",
        )?;
    }
    if let Some(pending) = &session.pending {
        corrupt_if(
            session.answered.len() == MAX_ANSWERED_AXES
                && !axes.contains(pending.question.axis.as_str()),
            "a full answer projection cannot accept the pending axis",
        )?;
    }
    Ok(())
}

#[cfg(windows)]
fn directory_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn directory_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn validate_brief(brief: &SessionBrief) -> Result<(), SessionError> {
    corrupt_if(brief.api != SESSION_API, "unknown brief API")?;
    corrupt_if(
        !is_session_id(&brief.session_id),
        "invalid brief session id",
    )?;
    require_decimal(&brief.turn, "brief turn")?;
    corrupt_if(brief.findings.len() > MAX_FINDINGS, "too many findings")?;
    for finding in &brief.findings {
        validate_text(&finding.summary, 400, true, "finding summary")?;
        validate_optional_text(finding.detail.as_deref(), 4_096, "finding detail")?;
        validate_optional_text(finding.source.as_deref(), 160, "finding source")?;
    }
    let question = &brief.question;
    corrupt_if(!is_axis_id(&question.axis), "invalid question axis")?;
    validate_text(&question.prompt, 1_024, true, "question prompt")?;
    corrupt_if(
        !(2..=MAX_OPTIONS).contains(&question.options.len()),
        "question must have two to six options",
    )?;
    let mut option_ids = BTreeSet::new();
    for option in &question.options {
        corrupt_if(!is_option_id(&option.id), "invalid option id")?;
        corrupt_if(
            !option_ids.insert(option.id.as_str()),
            "duplicate option id",
        )?;
        validate_text(&option.label, 160, true, "option label")?;
        corrupt_if(
            option.coordinates.len() > MAX_COORDINATES,
            "too many option coordinates",
        )?;
        for (key, value) in &option.coordinates {
            validate_text(key, 64, true, "coordinate key")?;
            validate_text(value, 64, true, "coordinate value")?;
        }
        validate_optional_text(option.rationale.as_deref(), 1_024, "option rationale")?;
    }
    validate_axis_list(&question.depends_on, "question dependencies")?;
    corrupt_if(brief.stakes.len() > MAX_OPTIONS, "too many consequences")?;
    for stake in &brief.stakes {
        corrupt_if(
            !option_ids.contains(stake.option.as_str()),
            "consequence references an unknown option",
        )?;
        validate_text(&stake.effect, 1_024, true, "consequence effect")?;
    }
    if let Some(default) = &brief.default_option {
        corrupt_if(
            !option_ids.contains(default.as_str()),
            "default references an unknown option",
        )?;
    }
    let surface = validate_axis_list(&brief.remaining_surface, "remaining surface")?;
    let floor = validate_axis_list(&brief.remaining_floor, "remaining floor")?;
    corrupt_if(
        !floor.is_subset(&surface),
        "remaining floor is not a subset of the surface",
    )?;
    Ok(())
}

fn validate_axis_list<'a>(
    values: &'a [String],
    name: &str,
) -> Result<BTreeSet<&'a str>, SessionError> {
    corrupt_if(
        values.len() > MAX_REMAINING_AXES,
        &format!("too many {name}"),
    )?;
    let mut unique = BTreeSet::new();
    for value in values {
        corrupt_if(!is_axis_id(value), &format!("invalid axis in {name}"))?;
        corrupt_if(
            !unique.insert(value.as_str()),
            &format!("duplicate axis in {name}"),
        )?;
    }
    Ok(unique)
}

fn lock_session(directory: &Path) -> Result<File, SessionError> {
    let path = directory.join("session.lock");
    let file = open_plain_auxiliary(&path, directory)?;
    let started = Instant::now();
    loop {
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => return Ok(file),
            Err(fs4::TryLockError::WouldBlock) if started.elapsed() < SESSION_LOCK_TIMEOUT => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(fs4::TryLockError::WouldBlock) => {
                return Err(SessionError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "session answer lock timed out",
                )));
            }
            Err(fs4::TryLockError::Error(error)) => return Err(SessionError::Io(error)),
        }
    }
}

fn append_transcript(
    directory: &Path,
    brief: &SessionBrief,
    answer: &AnsweredAxis,
    note: Option<&str>,
) -> Result<AnsweredAxis, SessionError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct TranscriptRecord<'a> {
        api: &'static str,
        #[serde(rename = "type")]
        kind: &'static str,
        answer_id: String,
        session_id: &'a str,
        turn: &'a str,
        brief: &'a SessionBrief,
        answer: &'a AnsweredAxis,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<&'a str>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StoredTranscriptRecord {
        api: String,
        #[serde(rename = "type")]
        kind: String,
        answer_id: String,
        session_id: String,
        turn: String,
        brief: SessionBrief,
        answer: AnsweredAxis,
        #[serde(default)]
        note: Option<String>,
    }

    let answer_id = format!("{}:{}:{}", brief.session_id, brief.turn, answer.axis);
    let record = TranscriptRecord {
        api: SESSION_API,
        kind: "answer",
        answer_id: answer_id.clone(),
        session_id: &brief.session_id,
        turn: &brief.turn,
        brief,
        answer,
        note,
    };
    let mut encoded = serde_json::to_vec(&record)
        .map_err(|error| SessionError::Corrupt(format!("cannot encode transcript: {error}")))?;
    encoded.push(b'\n');
    if encoded.len() as u64 > MAX_TRANSCRIPT_RECORD_BYTES {
        return Err(SessionError::Corrupt(
            "transcript record exceeds 2 MiB".to_owned(),
        ));
    }
    let path = directory.join("transcript.jsonl");
    let mut file = open_plain_auxiliary(&path, directory)?;
    ensure_named_file_identity(&file, &path)?;
    if let Some(last_line) = repair_and_read_last_transcript_line(&mut file)? {
        let value = crate::protocol::decode_json(&last_line).map_err(|error| {
            SessionError::Corrupt(format!("invalid complete transcript record: {error}"))
        })?;
        if value.get("answerId").and_then(serde_json::Value::as_str) == Some(&answer_id) {
            let stored: StoredTranscriptRecord =
                serde_json::from_value(value).map_err(|error| {
                    SessionError::Corrupt(format!("invalid stored answer record: {error}"))
                })?;
            if stored.api != SESSION_API
                || stored.kind != "answer"
                || stored.answer_id != answer_id
                || stored.session_id != brief.session_id
                || stored.turn != brief.turn
                || stored.brief != *brief
                || stored.answer.axis != answer.axis
                || stored.answer.option != answer.option
                || stored.answer.label != answer.label
                || stored.answer.defaulted != answer.defaulted
                || stored.note.as_deref() != note
            {
                return Err(SessionError::InvalidArgument);
            }
            require_decimal(&stored.answer.answered_at_unix_ms, "transcript answer time")?;
            return Ok(stored.answer);
        }
    }
    ensure_named_file_identity(&file, &path)?;
    file.seek(SeekFrom::End(0)).map_err(SessionError::Io)?;
    file.write_all(&encoded).map_err(SessionError::Io)?;
    file.sync_data().map_err(SessionError::Io)?;
    Ok(answer.clone())
}

fn repair_and_read_last_transcript_line(file: &mut File) -> Result<Option<Vec<u8>>, SessionError> {
    let mut length = file.metadata().map_err(SessionError::Io)?.len();
    if length == 0 {
        return Ok(None);
    }

    file.seek(SeekFrom::Start(length - 1))
        .map_err(SessionError::Io)?;
    let mut final_byte = [0u8; 1];
    file.read_exact(&mut final_byte).map_err(SessionError::Io)?;
    if final_byte[0] != b'\n' {
        length = find_previous_newline(file, length)?.map_or(0, |position| position + 1);
        file.set_len(length).map_err(SessionError::Io)?;
        file.sync_data().map_err(SessionError::Io)?;
    }
    if length == 0 {
        return Ok(None);
    }

    let line_end = length - 1;
    let line_start = find_previous_newline(file, line_end)?.map_or(0, |position| position + 1);
    let line_length = line_end - line_start;
    if line_length == 0 || line_length > MAX_TRANSCRIPT_RECORD_BYTES {
        return Err(SessionError::Corrupt(
            "invalid bounded transcript record".to_owned(),
        ));
    }
    let mut line = vec![0u8; line_length as usize];
    file.seek(SeekFrom::Start(line_start))
        .map_err(SessionError::Io)?;
    file.read_exact(&mut line).map_err(SessionError::Io)?;
    Ok(Some(line))
}

fn find_previous_newline(file: &mut File, before: u64) -> Result<Option<u64>, SessionError> {
    let mut end = before;
    let lower_bound = before.saturating_sub(MAX_TRANSCRIPT_RECORD_BYTES + 1);
    let mut buffer = [0u8; 8 * 1024];
    while end > lower_bound {
        let start = end.saturating_sub(buffer.len() as u64).max(lower_bound);
        let count = (end - start) as usize;
        file.seek(SeekFrom::Start(start))
            .map_err(SessionError::Io)?;
        file.read_exact(&mut buffer[..count])
            .map_err(SessionError::Io)?;
        if let Some(index) = buffer[..count].iter().rposition(|byte| *byte == b'\n') {
            return Ok(Some(start + index as u64));
        }
        end = start;
    }
    if before <= MAX_TRANSCRIPT_RECORD_BYTES {
        Ok(None)
    } else {
        Err(SessionError::Corrupt(
            "transcript tail exceeds the 2 MiB recovery bound".to_owned(),
        ))
    }
}

fn open_existing_plain(
    path: &Path,
    expected_parent: &Path,
    missing_message: &str,
) -> Result<File, SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SessionError::Corrupt(missing_message.to_owned())
        } else {
            SessionError::Io(error)
        }
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(SessionError::Corrupt(
            "session path is not a plain file".to_owned(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options.open(path).map_err(SessionError::Io)?;
    validate_open_plain_file(&file, path, expected_parent)?;
    Ok(file)
}

fn open_plain_auxiliary(path: &Path, expected_parent: &Path) -> Result<File, SessionError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    configure_no_follow(&mut options);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(SessionError::Io)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(SessionError::Corrupt(
                    "session auxiliary path is not a plain file".to_owned(),
                ));
            }
            let mut existing = OpenOptions::new();
            existing.read(true).write(true);
            configure_no_follow(&mut existing);
            existing.open(path).map_err(SessionError::Io)?
        }
        Err(error) => return Err(SessionError::Io(error)),
    };
    validate_open_plain_file(&file, path, expected_parent)?;
    Ok(file)
}

fn validate_open_plain_file(
    file: &File,
    path: &Path,
    expected_parent: &Path,
) -> Result<(), SessionError> {
    let metadata = file.metadata().map_err(SessionError::Io)?;
    if !metadata.is_file() || open_file_is_reparse_point(file)? {
        return Err(SessionError::Corrupt(
            "session path is not a plain file".to_owned(),
        ));
    }
    if open_file_link_count(file)? != 1 {
        return Err(SessionError::Corrupt(
            "session file has multiple hard links".to_owned(),
        ));
    }
    let named = fs::symlink_metadata(path).map_err(SessionError::Io)?;
    if !named.is_file() || named.file_type().is_symlink() {
        return Err(SessionError::Corrupt(
            "session path is not a plain file".to_owned(),
        ));
    }
    let resolved = dunce::canonicalize(path).map_err(SessionError::Io)?;
    let expected_parent = dunce::canonicalize(expected_parent).map_err(SessionError::Io)?;
    if resolved.parent() != Some(expected_parent.as_path()) {
        return Err(SessionError::Corrupt(
            "session file escaped its directory".to_owned(),
        ));
    }
    ensure_named_file_identity(file, path)
}

#[cfg(unix)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(nix::libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn configure_no_follow(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn open_file_is_reparse_point(_file: &File) -> Result<bool, SessionError> {
    Ok(false)
}

#[cfg(windows)]
fn open_file_is_reparse_point(file: &File) -> Result<bool, SessionError> {
    const FILE_ATTRIBUTE_REPARSE_POINT: u64 = 0x0000_0400;
    let information = winapi_util::file::information(file).map_err(SessionError::Io)?;
    Ok(information.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

#[cfg(not(any(unix, windows)))]
fn open_file_is_reparse_point(_file: &File) -> Result<bool, SessionError> {
    Ok(false)
}

#[cfg(unix)]
fn open_file_link_count(file: &File) -> Result<u64, SessionError> {
    use std::os::unix::fs::MetadataExt;

    Ok(file.metadata().map_err(SessionError::Io)?.nlink())
}

#[cfg(windows)]
fn open_file_link_count(file: &File) -> Result<u64, SessionError> {
    winapi_util::file::information(file)
        .map(|information| information.number_of_links())
        .map_err(SessionError::Io)
}

#[cfg(not(any(unix, windows)))]
fn open_file_link_count(_file: &File) -> Result<u64, SessionError> {
    Ok(1)
}

#[cfg(unix)]
fn ensure_named_file_identity(file: &File, path: &Path) -> Result<(), SessionError> {
    use std::os::unix::fs::MetadataExt;

    corrupt_if(
        open_file_link_count(file)? != 1,
        "session file acquired another hard link",
    )?;
    let named = fs::symlink_metadata(path).map_err(SessionError::Io)?;
    if !named.is_file() || named.file_type().is_symlink() {
        return Err(SessionError::Corrupt(
            "session file changed while it was open".to_owned(),
        ));
    }
    let opened = file.metadata().map_err(SessionError::Io)?;
    corrupt_if(
        opened.dev() != named.dev() || opened.ino() != named.ino(),
        "session file changed while it was open",
    )
}

#[cfg(windows)]
fn ensure_named_file_identity(file: &File, path: &Path) -> Result<(), SessionError> {
    corrupt_if(
        open_file_link_count(file)? != 1,
        "session file acquired another hard link",
    )?;
    let named_metadata = fs::symlink_metadata(path).map_err(SessionError::Io)?;
    if !named_metadata.is_file() || named_metadata.file_type().is_symlink() {
        return Err(SessionError::Corrupt(
            "session file changed while it was open".to_owned(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let named = options.open(path).map_err(SessionError::Io)?;
    if open_file_is_reparse_point(&named)? {
        return Err(SessionError::Corrupt(
            "session file changed while it was open".to_owned(),
        ));
    }
    let opened = winapi_util::file::information(file).map_err(SessionError::Io)?;
    let named = winapi_util::file::information(&named).map_err(SessionError::Io)?;
    corrupt_if(
        opened.volume_serial_number() != named.volume_serial_number()
            || opened.file_index() != named.file_index(),
        "session file changed while it was open",
    )
}

#[cfg(not(any(unix, windows)))]
fn ensure_named_file_identity(_file: &File, path: &Path) -> Result<(), SessionError> {
    let metadata = fs::symlink_metadata(path).map_err(SessionError::Io)?;
    corrupt_if(
        !metadata.is_file() || metadata.file_type().is_symlink(),
        "session file changed while it was open",
    )
}

fn replace_session(directory: &Path, session: &SessionView) -> Result<(), SessionError> {
    let encoded = serde_json::to_vec(session)
        .map_err(|error| SessionError::Corrupt(format!("cannot encode session: {error}")))?;
    if encoded.len() as u64 > MAX_SESSION_DOCUMENT_BYTES {
        return Err(SessionError::Corrupt(
            "session document exceeds 1 MiB".to_owned(),
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(SessionError::Io)?;
    temporary.write_all(&encoded).map_err(SessionError::Io)?;
    temporary.as_file().sync_all().map_err(SessionError::Io)?;
    temporary
        .persist(directory.join("session.json"))
        .map_err(|error| SessionError::Io(error.error))?;
    sync_parent_directory(directory)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(directory: &Path) -> Result<(), SessionError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(SessionError::Io)
}

#[cfg(windows)]
fn sync_parent_directory(directory: &Path) -> Result<(), SessionError> {
    use std::os::windows::fs::OpenOptionsExt;

    // Windows does not guarantee FlushFileBuffers support for directory
    // handles. Attempt it with backup semantics, but the durable file flush
    // above remains the enforceable boundary when the filesystem rejects it.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    if let Ok(directory) = options.open(directory) {
        let _ = directory.sync_all();
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_parent_directory(_directory: &Path) -> Result<(), SessionError> {
    Ok(())
}

fn unix_ms() -> Result<u64, SessionError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SessionError::Corrupt("system clock predates Unix epoch".to_owned()))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| SessionError::Corrupt("system clock exceeds u64 milliseconds".to_owned()))
}

fn ensure_list_projection_budget(list: &SessionList) -> Result<(), SessionError> {
    struct BudgetWriter {
        remaining: u64,
    }

    impl Write for BudgetWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() as u64 > self.remaining {
                return Err(io::Error::other("session list projection exceeds 8 MiB"));
            }
            self.remaining -= bytes.len() as u64;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    serde_json::to_writer(
        BudgetWriter {
            remaining: MAX_SESSION_LIST_BYTES,
        },
        list,
    )
    .map_err(|_| SessionError::Corrupt("session list projection exceeds 8 MiB".to_owned()))
}

fn decimal_u64(value: &str) -> u64 {
    require_decimal(value, "decimal").unwrap_or(0)
}

fn require_decimal(value: &str, name: &str) -> Result<u64, SessionError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SessionError::Corrupt(format!("invalid {name}")));
    }
    value
        .parse::<u64>()
        .map_err(|_| SessionError::Corrupt(format!("{name} exceeds u64")))
}

fn validate_optional_text(
    value: Option<&str>,
    maximum: usize,
    name: &str,
) -> Result<(), SessionError> {
    if let Some(value) = value {
        validate_text(value, maximum, false, name)?;
    }
    Ok(())
}

fn validate_text(
    value: &str,
    maximum: usize,
    nonempty: bool,
    name: &str,
) -> Result<(), SessionError> {
    corrupt_if(nonempty && value.is_empty(), &format!("{name} is empty"))?;
    corrupt_if(
        value.chars().count() > maximum,
        &format!("{name} is too long"),
    )
}

fn is_option_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_alphanumeric() || index > 0 && matches!(byte, b'.' | b'_' | b'-')
    })
}

fn is_session_id(value: &str) -> bool {
    value.len() <= 128
        && value.strip_prefix("session-").is_some_and(|tail| {
            !tail.is_empty()
                && tail
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_axis_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 || !value.is_ascii() {
        return false;
    }
    value.split('.').enumerate().all(|(index, part)| {
        !part.is_empty()
            && part.bytes().enumerate().all(|(offset, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit() && !(index == 0 && offset == 0)
                    || byte == b'-' && index > 0
            })
    })
}

fn corrupt_if(condition: bool, message: &str) -> Result<(), SessionError> {
    if condition {
        Err(SessionError::Corrupt(message.to_owned()))
    } else {
        Ok(())
    }
}

/// Failure while reading or changing a durable session.
#[derive(Debug, Error)]
pub enum SessionError {
    /// Workspace discovery failed.
    #[error("workspace unavailable: {0}")]
    Workspace(#[source] WorkspaceError),
    /// A bounded filesystem operation failed.
    #[error("session I/O failed: {0}")]
    Io(#[source] io::Error),
    /// The client supplied an unsafe identifier.
    #[error("invalid session id")]
    InvalidId,
    /// A bounded command argument is malformed.
    #[error("invalid session command argument")]
    InvalidArgument,
    /// The selected session does not exist.
    #[error("session not found")]
    NotFound,
    /// Persisted state failed typed or bounded validation.
    #[error("corrupt session: {0}")]
    Corrupt(String),
    /// The answer targets an older brief.
    #[error("session turn is stale")]
    TurnStale,
    /// The answer targets another axis.
    #[error("session axis does not match")]
    AxisMismatch,
    /// The option is not in the pending question.
    #[error("session option is invalid")]
    OptionInvalid,
    /// Only an awaiting-answer session can accept an answer.
    #[error("session is not awaiting an answer")]
    StateInvalid,
}

impl SessionError {
    /// Stable public code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidId => "session_invalid_id",
            Self::InvalidArgument => "session_invalid_argument",
            Self::NotFound => "session_not_found",
            Self::Corrupt(_) => "session_corrupt",
            Self::TurnStale => "session_turn_stale",
            Self::AxisMismatch => "session_axis_mismatch",
            Self::OptionInvalid => "session_option_invalid",
            Self::StateInvalid => "session_state_invalid",
            Self::Workspace(_) | Self::Io(_) => "session_io_failed",
        }
    }

    fn public_message(&self) -> &'static str {
        match self {
            Self::InvalidId => "The supplied session identifier is invalid.",
            Self::InvalidArgument => "A supplied session command argument is invalid.",
            Self::NotFound => "The selected session was not found.",
            Self::Corrupt(_) => "The selected session contains invalid or corrupt data.",
            Self::TurnStale => "The supplied turn does not match the session's current turn.",
            Self::AxisMismatch => "The supplied axis does not match the pending question.",
            Self::OptionInvalid => "The supplied option is not present in the pending question.",
            Self::StateInvalid => {
                "The selected session cannot accept an answer in its current state."
            }
            Self::Workspace(_) | Self::Io(_) => {
                "Tactus could not read or update the selected session."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc, thread};

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::*;
    use crate::workspace::initialize_workspace;

    fn initialized() -> (tempfile::TempDir, PathBuf) {
        let temporary = tempdir().expect("temporary directory");
        let sdk = temporary.path().join("sdk");
        fs::create_dir(&sdk).expect("sdk directory");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk manifest");
        let root = temporary.path().join("project");
        let initialized = initialize_workspace(&root, Some(&sdk)).expect("initialize workspace");
        assert!(initialized.workspace.sessions_path.is_dir());
        (temporary, root)
    }

    fn pending_document(session_id: &str) -> Value {
        json!({
            "api": SESSION_API,
            "sessionId": session_id,
            "label": "Desk build",
            "state": "awaiting_answer",
            "turn": "3",
            "pending": {
                "api": SESSION_API,
                "sessionId": session_id,
                "turn": "3",
                "findings": [{
                    "summary": "The frame constrains the top.",
                    "source": "fixture"
                }],
                "question": {
                    "axis": "desk.frame",
                    "prompt": "Which frame?",
                    "options": [
                        {"id":"fixed", "label":"Fixed", "coordinates":{"height":"fixed"}},
                        {"id":"sit-stand", "label":"Sit/stand", "coordinates":{"height":"adjustable"}}
                    ],
                    "reversibility": "irreversible",
                    "dependsOn": []
                },
                "stakes": [{
                    "option":"sit-stand",
                    "effect":"Caps the top thickness.",
                    "reversibility":"irreversible"
                }],
                "defaultOption":"fixed",
                "remainingSurface":["desk.frame"],
                "remainingFloor":["desk.frame"]
            },
            "answered": [{
                "axis":"desk.frame",
                "option":"sit-stand",
                "label":"Sit/stand",
                "defaulted":false,
                "answeredAtUnixMs":"90"
            }],
            "startedUnixMs":"1",
            "updatedUnixMs":"100",
            "plannerVersion":"fixture-v2"
        })
    }

    fn write_session(root: &Path, session_id: &str, document: &Value) -> PathBuf {
        let directory = root.join(".tactus/sessions").join(session_id);
        fs::create_dir(&directory).expect("session directory");
        fs::write(
            directory.join("session.json"),
            serde_json::to_vec(document).expect("session JSON"),
        )
        .expect("session document");
        directory
    }

    #[test]
    fn answer_is_right_biased_atomic_and_auditable() {
        let (_temporary, root) = initialized();
        let session_id = "session-desk-1";
        let directory = write_session(&root, session_id, &pending_document(session_id));

        let before = show(&root, session_id).expect("show session");
        assert_eq!(before.state, SessionState::AwaitingAnswer);
        let after = answer(
            &root,
            session_id,
            "3",
            "desk.frame",
            "fixed",
            Some("Prefer the simpler build."),
        )
        .expect("answer session");
        assert_eq!(after.state, SessionState::Planning);
        assert!(after.pending.is_none());
        assert_eq!(after.answered.len(), 1);
        assert_eq!(after.answered[0].option, "fixed");
        assert_eq!(after.extensions["plannerVersion"], "fixture-v2");

        let persisted = show(&root, session_id).expect("show updated session");
        assert_eq!(persisted, after);
        let transcript =
            fs::read_to_string(directory.join("transcript.jsonl")).expect("transcript");
        let lines = transcript.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 1);
        let record: Value = serde_json::from_str(lines[0]).expect("transcript record");
        assert_eq!(record["answerId"], "session-desk-1:3:desk.frame");
        assert_eq!(record["note"], "Prefer the simpler build.");
        assert_eq!(record["brief"]["question"]["axis"], "desk.frame");
    }

    #[test]
    fn answer_rejects_stale_axis_option_and_invalid_state_without_writing() {
        let (_temporary, root) = initialized();
        let session_id = "session-validation";
        let directory = write_session(&root, session_id, &pending_document(session_id));

        let cases = [
            ("2", "desk.frame", "fixed", "session_turn_stale"),
            ("3", "desk.top", "fixed", "session_axis_mismatch"),
            ("3", "desk.frame", "unknown", "session_option_invalid"),
        ];
        for (turn, axis, option, code) in cases {
            let error = answer(&root, session_id, turn, axis, option, None).expect_err(code);
            assert_eq!(error.code(), code);
        }
        assert!(!directory.join("transcript.jsonl").exists());

        let mut planning = pending_document(session_id);
        planning["state"] = json!("planning");
        planning.as_object_mut().expect("object").remove("pending");
        fs::write(
            directory.join("session.json"),
            serde_json::to_vec(&planning).expect("planning JSON"),
        )
        .expect("planning document");
        let error =
            answer(&root, session_id, "3", "desk.frame", "fixed", None).expect_err("invalid state");
        assert_eq!(error.code(), "session_state_invalid");
    }

    #[test]
    fn concurrent_answers_commit_once_under_the_turn_lock() {
        let (_temporary, root) = initialized();
        let session_id = "session-concurrent";
        let directory = write_session(&root, session_id, &pending_document(session_id));
        let root = Arc::new(root);
        let workers = ["fixed", "sit-stand"].map(|option| {
            let root = Arc::clone(&root);
            thread::spawn(move || {
                answer(&root, session_id, "3", "desk.frame", option, Some(option))
            })
        });
        let results = workers.map(|worker| worker.join().expect("answer worker"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .map(SessionError::code)
                .collect::<Vec<_>>(),
            vec!["session_state_invalid"]
        );
        let transcript =
            fs::read_to_string(directory.join("transcript.jsonl")).expect("transcript");
        assert_eq!(transcript.lines().count(), 1);
    }

    #[test]
    fn documents_enforce_parent_references_and_collection_invariants() {
        let (_temporary, root) = initialized();
        let session_id = "session-corrupt";
        let mut document = pending_document(session_id);
        document["pending"]["sessionId"] = json!("session-other");
        write_session(&root, session_id, &document);
        let error = show(&root, session_id).expect_err("parent mismatch");
        assert_eq!(error.code(), "session_corrupt");

        assert_eq!(
            show(&root, "../escape").expect_err("invalid id").code(),
            "session_invalid_id"
        );
        assert_eq!(
            show(&root, "session-missing")
                .expect_err("missing session")
                .code(),
            "session_not_found"
        );
    }

    #[test]
    fn documents_match_bundle_axis_coordinate_and_stakes_rules() {
        let (_temporary, root) = initialized();

        let axis_session = "session-bundle-axis";
        let mut axis_document = pending_document(axis_session);
        axis_document["pending"]["question"]["axis"] = json!("desk.-frame");
        axis_document["pending"]["remainingSurface"] = json!(["desk.-frame"]);
        axis_document["pending"]["remainingFloor"] = json!(["desk.-frame"]);
        write_session(&root, axis_session, &axis_document);
        show(&root, axis_session).expect("later axis segment may start with a hyphen");

        let coordinate_session = "session-empty-coordinate";
        let mut coordinate_document = pending_document(coordinate_session);
        coordinate_document["pending"]["question"]["options"][0]["coordinates"]["height"] =
            json!("");
        write_session(&root, coordinate_session, &coordinate_document);
        assert_eq!(
            show(&root, coordinate_session)
                .expect_err("empty coordinate value")
                .code(),
            "session_corrupt"
        );

        let stakes_session = "session-multiple-stakes";
        let mut stakes_document = pending_document(stakes_session);
        let duplicate = stakes_document["pending"]["stakes"][0].clone();
        stakes_document["pending"]["stakes"] = json!([duplicate.clone(), duplicate]);
        write_session(&root, stakes_session, &stakes_document);
        show(&root, stakes_session).expect("one option may have multiple consequences");
    }

    #[test]
    fn answer_is_monotonic_and_refuses_a_257th_axis() {
        let (_temporary, root) = initialized();

        let future_session = "session-future-clock";
        let mut future = pending_document(future_session);
        let future_time = "18446744073709551600";
        future["updatedUnixMs"] = json!(future_time);
        write_session(&root, future_session, &future);
        let answered = answer(&root, future_session, "3", "desk.frame", "fixed", None)
            .expect("answer with a clock behind persisted state");
        assert_eq!(answered.updated_unix_ms, future_time);
        assert_eq!(answered.answered[0].answered_at_unix_ms, future_time);

        let full_session = "session-full-axes";
        let mut full = pending_document(full_session);
        full["pending"]["question"]["axis"] = json!("desk.new-axis");
        full["pending"]["remainingSurface"] = json!(["desk.new-axis"]);
        full["pending"]["remainingFloor"] = json!(["desk.new-axis"]);
        full["answered"] = Value::Array(
            (0..MAX_ANSWERED_AXES)
                .map(|index| {
                    json!({
                        "axis": format!("history.axis{index}"),
                        "option": "fixed",
                        "label": "Fixed",
                        "defaulted": false,
                        "answeredAtUnixMs": "90"
                    })
                })
                .collect(),
        );
        let directory = write_session(&root, full_session, &full);
        let error = answer(&root, full_session, "3", "desk.new-axis", "fixed", None)
            .expect_err("257th answer axis");
        assert_eq!(error.code(), "session_corrupt");
        assert!(!directory.join("transcript.jsonl").exists());
    }

    #[test]
    fn transcript_repairs_partial_tail_and_recovers_an_orphan_once() {
        let (_temporary, root) = initialized();
        let session_id = "session-transcript-recovery";
        let directory = write_session(&root, session_id, &pending_document(session_id));
        let pending = show(&root, session_id)
            .expect("pending session")
            .pending
            .expect("pending brief");
        let orphan = AnsweredAxis {
            axis: "desk.frame".to_owned(),
            option: "fixed".to_owned(),
            label: "Fixed".to_owned(),
            defaulted: false,
            answered_at_unix_ms: "100".to_owned(),
            extensions: BTreeMap::new(),
        };
        append_transcript(
            &directory,
            &pending,
            &orphan,
            Some("Recover this exact choice."),
        )
        .expect("orphan transcript append");
        OpenOptions::new()
            .append(true)
            .open(directory.join("transcript.jsonl"))
            .and_then(|mut file| file.write_all(br#"{"partial"#))
            .expect("partial crash tail");

        let recovered = answer(
            &root,
            session_id,
            "3",
            "desk.frame",
            "fixed",
            Some("Recover this exact choice."),
        )
        .expect("idempotent recovery");
        assert_eq!(recovered.answered[0].answered_at_unix_ms, "100");
        let transcript =
            fs::read_to_string(directory.join("transcript.jsonl")).expect("repaired transcript");
        assert!(transcript.ends_with('\n'));
        assert_eq!(transcript.lines().count(), 1);
        assert!(!transcript.contains("partial"));
    }

    #[test]
    fn transcript_recovery_never_scans_or_truncates_an_oversized_tail() {
        let temporary = tempdir().expect("temporary transcript directory");
        for (name, prefix) in [("no-newline", &b""[..]), ("partial-tail", &b"{}\n"[..])] {
            let path = temporary.path().join(name);
            let mut original = prefix.to_vec();
            original.extend(std::iter::repeat_n(
                b'x',
                MAX_TRANSCRIPT_RECORD_BYTES as usize + 1,
            ));
            fs::write(&path, &original).expect("oversized transcript fixture");
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .expect("open oversized transcript");
            let error = repair_and_read_last_transcript_line(&mut file)
                .expect_err("oversized recovery tail");
            assert_eq!(error.code(), "session_corrupt");
            drop(file);
            let unchanged = fs::read(&path).expect("unchanged oversized transcript");
            assert_eq!(unchanged.len(), original.len());
            assert_eq!(unchanged, original);
        }
    }

    #[test]
    fn auxiliary_hard_links_are_rejected_before_writing() {
        let (temporary, root) = initialized();
        let session_id = "session-hard-link";
        let directory = write_session(&root, session_id, &pending_document(session_id));
        let external = temporary.path().join("outside-transcript.txt");
        fs::write(&external, "must remain unchanged\n").expect("external file");
        fs::hard_link(&external, directory.join("transcript.jsonl")).expect("hard link fixture");

        let error = answer(&root, session_id, "3", "desk.frame", "fixed", None)
            .expect_err("hard-linked transcript");
        assert_eq!(error.code(), "session_corrupt");
        assert_eq!(
            fs::read_to_string(external).expect("external remains readable"),
            "must remain unchanged\n"
        );
    }

    #[test]
    fn option_labels_arguments_and_directory_case_are_validated_consistently() {
        let (_temporary, root) = initialized();

        let identity_session = "session-option-identity";
        let mut invalid_option = pending_document(identity_session);
        invalid_option["pending"]["question"]["options"][0]["id"] = json!("-fixed");
        write_session(&root, identity_session, &invalid_option);
        assert_eq!(
            show(&root, identity_session)
                .expect_err("leading-hyphen stored option")
                .code(),
            "session_corrupt"
        );

        let answered_session = "session-answered-identity";
        let mut invalid_answered = pending_document(answered_session);
        invalid_answered["answered"][0]["option"] = json!("-fixed");
        write_session(&root, answered_session, &invalid_answered);
        assert_eq!(
            show(&root, answered_session)
                .expect_err("leading-hyphen historical option")
                .code(),
            "session_corrupt"
        );

        let label_session = "session-empty-label";
        let mut empty_label = pending_document(label_session);
        empty_label["label"] = json!("");
        write_session(&root, label_session, &empty_label);
        assert_eq!(
            show(&root, label_session)
                .expect_err("empty session label")
                .code(),
            "session_corrupt"
        );

        let argument_session = "session-invalid-arguments";
        write_session(&root, argument_session, &pending_document(argument_session));
        assert_eq!(
            answer(&root, argument_session, "03", "desk.frame", "fixed", None)
                .expect_err("non-canonical turn")
                .code(),
            "session_invalid_argument"
        );
        assert_eq!(
            answer(
                &root,
                argument_session,
                "3",
                "desk.frame",
                "fixed",
                Some("contains\0nul")
            )
            .expect_err("NUL note")
            .code(),
            "session_invalid_argument"
        );
        assert_eq!(
            answer(&root, argument_session, "3", "desk.frame", "-fixed", None)
                .expect_err("invalid option tag")
                .code(),
            "session_option_invalid"
        );
        assert_eq!(
            list(&root, 0).expect_err("zero limit").code(),
            "session_invalid_argument"
        );

        let mixed_case = "session-MixedCase";
        write_session(&root, mixed_case, &pending_document(mixed_case));
        show(&root, mixed_case).expect("exact mixed-case session id");
        assert_eq!(
            show(&root, "session-mixedcase")
                .expect_err("wrong-case session id")
                .code(),
            "session_not_found"
        );
    }

    #[test]
    fn list_scales_across_a_large_directory_and_retains_only_the_newest_page() {
        const SESSION_COUNT: usize = 512;
        const PAGE_SIZE: usize = 7;

        let (_temporary, root) = initialized();
        for index in 0..SESSION_COUNT {
            let session_id = format!("session-scale-{index:04}");
            let mut document = pending_document(&session_id);
            // Adjacent pairs deliberately tie so the session-id tiebreaker is
            // exercised at the top-K boundary as well as within the page.
            document["updatedUnixMs"] = json!((index / 2 + 100).to_string());
            write_session(&root, &session_id, &document);
        }

        let started = Instant::now();
        let page = list(&root, PAGE_SIZE).expect("bounded large-directory listing");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a 512-entry listing should remain comfortably bounded"
        );
        assert_eq!(
            page.sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "session-scale-0510",
                "session-scale-0511",
                "session-scale-0508",
                "session-scale-0509",
                "session-scale-0506",
                "session-scale-0507",
                "session-scale-0504",
            ]
        );
    }

    #[test]
    fn list_still_validates_candidates_that_fall_outside_the_requested_page() {
        let (_temporary, root) = initialized();
        let newest_id = "session-newest";
        let mut newest = pending_document(newest_id);
        newest["updatedUnixMs"] = json!("1000");
        write_session(&root, newest_id, &newest);

        let corrupt_id = "session-old-corrupt";
        let mut corrupt = pending_document(corrupt_id);
        corrupt["updatedUnixMs"] = json!("2");
        corrupt["label"] = json!("");
        write_session(&root, corrupt_id, &corrupt);

        let error = list(&root, 1).expect_err("discarded candidates remain validated");
        assert_eq!(error.code(), "session_corrupt");
    }

    #[test]
    fn list_enforces_the_eight_mib_document_budget() {
        let (_temporary, root) = initialized();
        for index in 0..9 {
            let session_id = format!("session-large-{index}");
            let mut document = pending_document(&session_id);
            document["largeProducerField"] = json!("x".repeat(950 * 1024));
            write_session(&root, &session_id, &document);
        }
        let error = list(&root, MAX_LIST_LIMIT).expect_err("8 MiB aggregate budget");
        assert_eq!(error.code(), "session_corrupt");
    }

    #[cfg(windows)]
    #[test]
    fn windows_junctions_cannot_stand_in_for_session_directories() {
        fn junction(link: &Path, target: &Path) {
            let output = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "New-Item -ItemType Junction -Path $env:TACTUS_TEST_JUNCTION -Target $env:TACTUS_TEST_TARGET | Out-Null",
                ])
                .env("TACTUS_TEST_JUNCTION", link)
                .env("TACTUS_TEST_TARGET", target)
                .output()
                .expect("create junction");
            assert!(
                output.status.success(),
                "mklink failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let (_temporary, root) = initialized();
        let sessions = root.join(".tactus/sessions");
        fs::remove_dir(&sessions).expect("empty sessions directory");
        let replacement = root.join(".tactus/session-storage");
        fs::create_dir(&replacement).expect("replacement sessions directory");
        junction(&sessions, &replacement);
        assert_eq!(
            list(&root, 1).expect_err("sessions junction").code(),
            "session_corrupt"
        );

        let (_temporary, root) = initialized();
        let sessions = root.join(".tactus/sessions");
        let backing = sessions.join("backing-storage");
        fs::create_dir(&backing).expect("backing session directory");
        let session_id = "session-junction";
        fs::write(
            backing.join("session.json"),
            serde_json::to_vec(&pending_document(session_id)).expect("session JSON"),
        )
        .expect("session document");
        junction(&sessions.join(session_id), &backing);
        assert_eq!(
            show(&root, session_id)
                .expect_err("session directory junction")
                .code(),
            "session_corrupt"
        );
    }
}
