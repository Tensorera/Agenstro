//! Append-only run events and atomically published summaries.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{process::ProcessOutcome, workspace::Workspace};

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
    /// Structured factual payload.
    pub data: Value,
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
    events: BufWriter<File>,
    finished: bool,
}

impl RunJournal {
    /// Create a unique run directory below `.tactus/runs`.
    pub fn create(workspace: &Workspace) -> Result<Self, JournalError> {
        fs::create_dir_all(&workspace.runs_path).map_err(JournalError::Io)?;
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
            .map_err(JournalError::Io)?;
        let summary_path = run_path.join("summary.json");
        Ok(Self {
            run_id,
            run_path,
            event_path,
            summary_path,
            started_unix_ms,
            next_seq: 1,
            events,
            finished: false,
        })
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
            data,
        };
        serde_json::to_writer(&mut self.events, &event).map_err(JournalError::Json)?;
        self.events.write_all(b"\n").map_err(JournalError::Io)?;
        self.events.flush().map_err(JournalError::Io)?;
        self.next_seq += 1;
        Ok(seq)
    }

    /// Flush events and atomically publish `summary.json`.
    pub fn finish(&mut self, outcome: ProcessOutcome) -> Result<RunSummary, JournalError> {
        if self.finished {
            return Err(JournalError::AlreadyFinished);
        }
        self.events.flush().map_err(JournalError::Io)?;
        self.events.get_ref().sync_all().map_err(JournalError::Io)?;
        let summary = RunSummary {
            api: TRACE_API.to_owned(),
            run_id: self.run_id.clone(),
            started_unix_ms: self.started_unix_ms,
            finished_unix_ms: unix_millis(),
            events_recorded: self.next_seq - 1,
            outcome,
        };
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
            serde_json::to_writer(&mut file, &summary).map_err(JournalError::Json)?;
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
        self.finished = true;
        Ok(summary)
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
            terminal: None,
            frames_seen: 1,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms: 1,
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
}
