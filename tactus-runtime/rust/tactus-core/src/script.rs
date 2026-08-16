use std::collections::BTreeSet;

use agentro_contracts::Sha256Digest;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{CellId, TactusIdError};

/// Maximum bytes accepted from one direct Tactus script.
pub const MAX_SCRIPT_BYTES: usize = 1024 * 1024;
/// Maximum cells accepted from one direct Tactus script.
pub const MAX_SCRIPT_CELLS: usize = 10_000;
/// Maximum UTF-8 bytes in one cell title.
pub const MAX_CELL_TITLE_BYTES: usize = 512;

const CELL_ID_PREFIX: &str = "[tactus-cell:";

/// Whether a direct-script cell executes Python or projects notes only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptCellKind {
    /// Independently executable Python source.
    Code,
    /// Non-executable Markdown-style notes.
    Markdown,
}

/// One direct-script cell with identity independent of position and source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptCell {
    id: CellId,
    ordinal: u32,
    kind: ScriptCellKind,
    title: String,
    source: String,
    source_digest: Sha256Digest,
    start_line: u32,
    end_line: u32,
}

impl ScriptCell {
    /// Returns the durable UUID embedded in the marker.
    #[must_use]
    pub const fn id(&self) -> CellId {
        self.id
    }

    /// Returns the current one-based display order.
    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    /// Returns whether this is code or projected notes.
    #[must_use]
    pub const fn kind(&self) -> ScriptCellKind {
        self.kind
    }

    /// Returns the bounded display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns source excluding the `# %%` marker.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the normalized source digest, separate from cell identity.
    #[must_use]
    pub const fn source_digest(&self) -> Sha256Digest {
        self.source_digest
    }

    /// Returns the first one-based source line in the input script.
    #[must_use]
    pub const fn start_line(&self) -> u32 {
        self.start_line
    }

    /// Returns the last one-based source line in the input script.
    #[must_use]
    pub const fn end_line(&self) -> u32 {
        self.end_line
    }

    /// Reports whether the cell contains executable code.
    #[must_use]
    pub fn is_executable(&self) -> bool {
        self.kind == ScriptCellKind::Code && !self.source.trim().is_empty()
    }
}

/// A script whose every executable boundary carries a stable cell UUID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedScript {
    source: String,
    cells: Vec<ScriptCell>,
}

impl NormalizedScript {
    /// Returns source with canonical stable-ID markers.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns cells in current script order.
    #[must_use]
    pub fn cells(&self) -> &[ScriptCell] {
        &self.cells
    }
}

/// Direct-script marker, identity, or resource-bound failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ScriptError {
    /// Script text exceeded [`MAX_SCRIPT_BYTES`].
    #[error("direct script exceeds its byte limit")]
    SourceTooLarge,
    /// The script contained more than [`MAX_SCRIPT_CELLS`] cells.
    #[error("direct script exceeds its cell limit")]
    TooManyCells,
    /// Parsing requires markers previously normalized with stable IDs.
    #[error("direct script must be normalized to assign stable cell IDs")]
    NeedsNormalization,
    /// A stable marker had malformed syntax or UUID text.
    #[error("direct script contains an invalid stable cell marker")]
    InvalidMarker,
    /// Two markers used the same stable cell UUID.
    #[error("direct script contains duplicate cell ID {cell_id}")]
    DuplicateCellId {
        /// Repeated identity.
        cell_id: CellId,
    },
    /// A cell title exceeded [`MAX_CELL_TITLE_BYTES`].
    #[error("direct script cell title exceeds its byte limit")]
    TitleTooLong,
    /// A line or ordinal cannot fit the durable 32-bit representation.
    #[error("direct script line or cell count cannot be represented")]
    CountOverflow,
}

/// Assigns missing stable IDs and returns canonical `# %%` markers.
///
/// Existing IDs are preserved across source edits and cell movement. A plain
/// Python file becomes one cell; a non-empty preamble before the first marker
/// becomes its own cell. Copying a canonical marker without changing its UUID
/// is rejected as a duplicate.
///
/// # Errors
///
/// Returns [`ScriptError`] for malformed markers, duplicate IDs, or bounds.
pub fn normalize_script(source: &str) -> Result<NormalizedScript, ScriptError> {
    compile_script(source, true)
}

/// Parses a script that already contains canonical stable-ID markers.
///
/// # Errors
///
/// Returns [`ScriptError::NeedsNormalization`] for plain or legacy markers and
/// another [`ScriptError`] for malformed, duplicate, or excessive input.
pub fn parse_script(source: &str) -> Result<NormalizedScript, ScriptError> {
    compile_script(source, false)
}

#[derive(Clone, Debug)]
struct Marker {
    line: usize,
    indentation: String,
    metadata: String,
    line_ending: &'static str,
}

#[derive(Clone, Debug)]
struct ParsedMarker {
    id: CellId,
    kind: ScriptCellKind,
    title: String,
}

fn compile_script(source: &str, assign_missing: bool) -> Result<NormalizedScript, ScriptError> {
    if source.len() > MAX_SCRIPT_BYTES {
        return Err(ScriptError::SourceTooLarge);
    }
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let markers = scan_markers(&lines);
    if markers.len() > MAX_SCRIPT_CELLS {
        return Err(ScriptError::TooManyCells);
    }

    if markers.is_empty() {
        if !assign_missing {
            return Err(ScriptError::NeedsNormalization);
        }
        let id = CellId::generate();
        let mut normalized = format!("# %% {CELL_ID_PREFIX}{id}] Script\n");
        normalized.push_str(source);
        return Ok(NormalizedScript {
            source: normalized,
            cells: vec![make_cell(
                id,
                1,
                ScriptCellKind::Code,
                "Script",
                source,
                1,
                lines.len(),
            )?],
        });
    }

    let first_marker_line = markers[0].line;
    let preamble = lines[..first_marker_line].concat();
    let has_preamble = !preamble.trim().is_empty();
    if has_preamble && !assign_missing {
        return Err(ScriptError::NeedsNormalization);
    }
    let expected_cells = markers.len().saturating_add(usize::from(has_preamble));
    if expected_cells > MAX_SCRIPT_CELLS {
        return Err(ScriptError::TooManyCells);
    }

    let mut seen = BTreeSet::new();
    let mut parsed_markers = Vec::with_capacity(markers.len());
    for marker in &markers {
        let parsed = parse_marker(&marker.metadata, assign_missing)?;
        if !seen.insert(parsed.id) {
            return Err(ScriptError::DuplicateCellId { cell_id: parsed.id });
        }
        parsed_markers.push(parsed);
    }

    let preamble_id = if has_preamble {
        let id = CellId::generate();
        if !seen.insert(id) {
            return Err(ScriptError::DuplicateCellId { cell_id: id });
        }
        Some(id)
    } else {
        None
    };

    let mut normalized = String::with_capacity(source.len().saturating_add(expected_cells * 64));
    if let Some(id) = preamble_id {
        normalized.push_str(&format!("# %% {CELL_ID_PREFIX}{id}] Preamble\n"));
    }
    let mut marker_index = 0_usize;
    for (line_index, line) in lines.iter().enumerate() {
        if markers
            .get(marker_index)
            .is_some_and(|marker| marker.line == line_index)
        {
            let marker = &markers[marker_index];
            let parsed = &parsed_markers[marker_index];
            normalized.push_str(&marker.indentation);
            normalized.push_str("# %% ");
            normalized.push_str(CELL_ID_PREFIX);
            normalized.push_str(&parsed.id.to_string());
            normalized.push(']');
            if parsed.kind == ScriptCellKind::Markdown {
                normalized.push_str(" [markdown]");
            }
            if !parsed.title.is_empty() {
                normalized.push(' ');
                normalized.push_str(&parsed.title);
            }
            normalized.push_str(marker.line_ending);
            marker_index = marker_index.saturating_add(1);
        } else {
            normalized.push_str(line);
        }
    }

    let mut cells = Vec::with_capacity(expected_cells);
    if let Some(id) = preamble_id {
        cells.push(make_cell(
            id,
            1,
            ScriptCellKind::Code,
            "Preamble",
            &preamble,
            1,
            first_marker_line,
        )?);
    }
    for (index, marker) in markers.iter().enumerate() {
        let source_start = marker.line.saturating_add(1);
        let source_end = markers
            .get(index.saturating_add(1))
            .map_or(lines.len(), |next| next.line);
        let cell_source = lines[source_start..source_end].concat();
        let parsed = &parsed_markers[index];
        let ordinal = cells.len().saturating_add(1);
        cells.push(make_cell(
            parsed.id,
            ordinal,
            parsed.kind,
            &parsed.title,
            &cell_source,
            source_start.saturating_add(1),
            source_end.max(source_start.saturating_add(1)),
        )?);
    }

    Ok(NormalizedScript {
        source: normalized,
        cells,
    })
}

#[allow(clippy::too_many_arguments)]
fn make_cell(
    id: CellId,
    ordinal: usize,
    kind: ScriptCellKind,
    title: &str,
    source: &str,
    start_line: usize,
    end_line: usize,
) -> Result<ScriptCell, ScriptError> {
    Ok(ScriptCell {
        id,
        ordinal: u32::try_from(ordinal).map_err(|_| ScriptError::CountOverflow)?,
        kind,
        title: title.to_owned(),
        source: source.to_owned(),
        source_digest: normalized_source_digest(source),
        start_line: u32::try_from(start_line).map_err(|_| ScriptError::CountOverflow)?,
        end_line: u32::try_from(end_line).map_err(|_| ScriptError::CountOverflow)?,
    })
}

fn parse_marker(metadata: &str, assign_missing: bool) -> Result<ParsedMarker, ScriptError> {
    let mut remaining = metadata.trim();
    let id = if let Some(after_prefix) = remaining.strip_prefix(CELL_ID_PREFIX) {
        let Some((id_text, rest)) = after_prefix.split_once(']') else {
            return Err(ScriptError::InvalidMarker);
        };
        remaining = rest.trim_start();
        CellId::parse(id_text).map_err(map_id_error)?
    } else if remaining.contains(CELL_ID_PREFIX) {
        return Err(ScriptError::InvalidMarker);
    } else if assign_missing {
        CellId::generate()
    } else {
        return Err(ScriptError::NeedsNormalization);
    };

    let kind = if let Some(rest) = strip_markdown_prefix(remaining) {
        remaining = rest;
        ScriptCellKind::Markdown
    } else {
        ScriptCellKind::Code
    };
    let title = remaining.trim();
    if title.len() > MAX_CELL_TITLE_BYTES {
        return Err(ScriptError::TitleTooLong);
    }
    let title = if title.is_empty() {
        match kind {
            ScriptCellKind::Code => "Cell",
            ScriptCellKind::Markdown => "Notes",
        }
    } else {
        title
    };
    Ok(ParsedMarker {
        id,
        kind,
        title: title.to_owned(),
    })
}

fn map_id_error(_error: TactusIdError) -> ScriptError {
    ScriptError::InvalidMarker
}

fn strip_markdown_prefix(value: &str) -> Option<&str> {
    const PREFIX: &str = "[markdown]";
    let prefix = value.get(..PREFIX.len())?;
    if prefix.eq_ignore_ascii_case(PREFIX) {
        value.get(PREFIX.len()..).map(str::trim_start)
    } else {
        None
    }
}

fn normalized_source_digest(source: &str) -> Sha256Digest {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = if normalized.ends_with('\n') {
        normalized
    } else {
        format!("{normalized}\n")
    };
    Sha256Digest::from_bytes(Sha256::digest(normalized.as_bytes()).into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LexState {
    Normal,
    TripleSingle,
    TripleDouble,
}

fn scan_markers(lines: &[&str]) -> Vec<Marker> {
    let mut markers = Vec::new();
    let mut state = LexState::Normal;
    for (line_index, line) in lines.iter().enumerate() {
        if state == LexState::Normal
            && let Some(marker) = marker_from_line(line_index, line)
        {
            markers.push(marker);
            continue;
        }
        scan_python_line(line, &mut state);
    }
    markers
}

fn marker_from_line(line_index: usize, line: &str) -> Option<Marker> {
    let (body, line_ending) = if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    };
    let indentation_bytes = body
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    let after_indent = body.get(indentation_bytes..)?;
    let after_hash = after_indent
        .strip_prefix('#')?
        .trim_start_matches([' ', '\t']);
    let metadata = after_hash.strip_prefix("%%")?.trim().to_owned();
    Some(Marker {
        line: line_index,
        indentation: body[..indentation_bytes].to_owned(),
        metadata,
        line_ending,
    })
}

fn scan_python_line(line: &str, state: &mut LexState) {
    let bytes = line.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        match *state {
            LexState::TripleSingle | LexState::TripleDouble => {
                let quote = if *state == LexState::TripleSingle {
                    b'\''
                } else {
                    b'"'
                };
                if bytes[index] == b'\\' {
                    index = index.saturating_add(2);
                } else if bytes.get(index..index.saturating_add(3)) == Some(&[quote, quote, quote])
                {
                    *state = LexState::Normal;
                    index = index.saturating_add(3);
                } else {
                    index = index.saturating_add(1);
                }
            }
            LexState::Normal => match bytes[index] {
                b'#' => return,
                quote @ (b'\'' | b'"') => {
                    if bytes.get(index..index.saturating_add(3)) == Some(&[quote, quote, quote]) {
                        *state = if quote == b'\'' {
                            LexState::TripleSingle
                        } else {
                            LexState::TripleDouble
                        };
                        index = index.saturating_add(3);
                    } else {
                        index = scan_single_quoted(bytes, index.saturating_add(1), quote);
                    }
                }
                _ => index = index.saturating_add(1),
            },
        }
    }
}

fn scan_single_quoted(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = index.saturating_add(2);
        } else if bytes[index] == quote {
            return index.saturating_add(1);
        } else {
            index = index.saturating_add(1);
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_script_is_normalized_to_one_stable_cell() -> Result<(), ScriptError> {
        let source = "value = 40 + 2\nassert value == 42\n";
        let normalized = normalize_script(source)?;
        assert_eq!(normalized.cells().len(), 1);
        assert_eq!(normalized.cells()[0].source(), source);
        assert!(normalized.source().starts_with("# %% [tactus-cell:"));

        let reparsed = parse_script(normalized.source())?;
        assert_eq!(reparsed.cells()[0].id(), normalized.cells()[0].id());
        Ok(())
    }

    #[test]
    fn marker_inside_triple_quoted_text_does_not_split_a_cell() -> Result<(), ScriptError> {
        let source = concat!(
            "# %% Build text\n",
            "message = \"\"\"first\n",
            "# %% not a marker\n",
            "last\"\"\"\n",
            "# %% Use text\n",
            "assert message\n",
        );
        let normalized = normalize_script(source)?;
        assert_eq!(normalized.cells().len(), 2);
        assert!(normalized.cells()[0].source().contains("not a marker"));
        Ok(())
    }

    #[test]
    fn moving_and_editing_a_cell_preserves_id_but_changes_source_digest() -> Result<(), ScriptError>
    {
        let first = normalize_script("# %% One\nvalue = 1\n# %% Two\nvalue = 2\n")?;
        let one = &first.cells()[0];
        let two = &first.cells()[1];
        let moved = format!(
            "# %% {CELL_ID_PREFIX}{}] Two\nvalue = 20\n# %% {CELL_ID_PREFIX}{}] One\nvalue = 1\n",
            two.id(),
            one.id(),
        );
        let second = parse_script(&moved)?;
        assert_eq!(second.cells()[0].id(), two.id());
        assert_ne!(second.cells()[0].source_digest(), two.source_digest());
        assert_eq!(second.cells()[1].id(), one.id());
        Ok(())
    }

    #[test]
    fn copied_stable_marker_is_rejected() -> Result<(), ScriptError> {
        let id = CellId::generate();
        let source = format!(
            "# %% {CELL_ID_PREFIX}{id}] First\npass\n# %% {CELL_ID_PREFIX}{id}] Copy\npass\n"
        );
        assert_eq!(
            parse_script(&source),
            Err(ScriptError::DuplicateCellId { cell_id: id })
        );
        Ok(())
    }

    #[test]
    fn markdown_cells_are_identified_but_not_executable() -> Result<(), ScriptError> {
        let script = normalize_script("# %% [markdown] Design\n# notes\n# %% Run\nassert True\n")?;
        assert_eq!(script.cells()[0].kind(), ScriptCellKind::Markdown);
        assert!(!script.cells()[0].is_executable());
        assert!(script.cells()[1].is_executable());
        Ok(())
    }
}
