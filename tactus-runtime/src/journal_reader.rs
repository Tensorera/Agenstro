//! Shared wire validation for journal queries. Filesystem and presentation
//! policy stay with the caller; record boundaries and integrity do not.

use std::io::{self, BufRead};

use crate::{
    journal::{RunSummary, TRACE_API, TraceEvent},
    limits::MAX_FRAME_BYTES_CEILING,
    outcome::validate_outcome_consistency,
};

pub(crate) const MAX_EVENT_LINE_BYTES: usize = MAX_FRAME_BYTES_CEILING + 4 * 1024 * 1024;
pub(crate) const MAX_EVENT_SCAN_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_SUMMARY_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum ReadError {
    Io(io::Error),
    Json(serde_json::Error),
    Corrupt(&'static str),
    LineTooLarge,
    ScanBudgetExceeded,
}

pub(crate) struct BoundedLine {
    pub bytes: Vec<u8>,
    pub terminated: bool,
}

pub(crate) fn read_bounded_line(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> Result<Option<BoundedLine>, ReadError> {
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(ReadError::Io)?;
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
            return Err(ReadError::LineTooLarge);
        }
        let terminated = available[take - 1] == b'\n';
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

pub(crate) fn strip_newline(bytes: &[u8]) -> &[u8] {
    let without_lf = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

pub(crate) fn decode_summary(bytes: &[u8], run_id: &str) -> Result<RunSummary, ReadError> {
    let summary: RunSummary = serde_json::from_slice(bytes).map_err(ReadError::Json)?;
    if summary.api != TRACE_API || summary.run_id != run_id {
        return Err(ReadError::Corrupt(
            "summary identity does not match its run directory",
        ));
    }
    validate_outcome_consistency(&summary.outcome).map_err(ReadError::Corrupt)?;
    Ok(summary)
}

/// A partial record is permissible only while the journal has no summary.
pub(crate) enum EventRead {
    Event { event: TraceEvent, bytes: usize },
    End,
    Partial,
}

pub(crate) struct EventReader<'a, R> {
    reader: R,
    run_id: &'a str,
    closed: bool,
    count: u64,
    scanned_bytes: u64,
}

impl<'a, R: BufRead> EventReader<'a, R> {
    pub(crate) fn new(reader: R, run_id: &'a str, closed: bool) -> Self {
        Self {
            reader,
            run_id,
            closed,
            count: 0,
            scanned_bytes: 0,
        }
    }

    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    /// Call only after reaching EOF, since a bounded page need not contain the
    /// complete stream. Both query surfaces use the same count invariant.
    pub(crate) fn validate_summary_count(&self, summary: &RunSummary) -> Result<(), ReadError> {
        if summary.events_recorded != self.count {
            return Err(ReadError::Corrupt(
                "summary event count does not match events.jsonl",
            ));
        }
        Ok(())
    }

    pub(crate) fn next(&mut self) -> Result<EventRead, ReadError> {
        let Some(line) = read_bounded_line(&mut self.reader, MAX_EVENT_LINE_BYTES)? else {
            return Ok(EventRead::End);
        };
        self.scanned_bytes = self.scanned_bytes.saturating_add(line.bytes.len() as u64);
        if self.scanned_bytes > MAX_EVENT_SCAN_BYTES {
            return Err(ReadError::ScanBudgetExceeded);
        }
        if !line.terminated {
            return if self.closed {
                Err(ReadError::Corrupt(
                    "closed event stream ends with an incomplete record",
                ))
            } else {
                Ok(EventRead::Partial)
            };
        }
        let event: TraceEvent =
            serde_json::from_slice(strip_newline(&line.bytes)).map_err(ReadError::Json)?;
        if event.api != TRACE_API || event.run_id != self.run_id || event.seq != self.count + 1 {
            return Err(ReadError::Corrupt(
                "event identity or sequence is inconsistent",
            ));
        }
        self.count += 1;
        Ok(EventRead::Event {
            event,
            bytes: line.bytes.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn bounded_lines_preserve_unicode_and_reject_oversized_tails() {
        let mut input = Cursor::new("你好\u{2028}world\r\npartial".as_bytes());
        let first = read_bounded_line(&mut input, 128).unwrap().unwrap();
        assert!(first.terminated);
        assert_eq!(strip_newline(&first.bytes), "你好\u{2028}world".as_bytes());
        let tail = read_bounded_line(&mut input, 128).unwrap().unwrap();
        assert!(!tail.terminated);
        assert_eq!(tail.bytes, b"partial");
        assert!(matches!(
            read_bounded_line(&mut Cursor::new(vec![b'x'; 65]), 32),
            Err(ReadError::LineTooLarge)
        ));
    }
}
