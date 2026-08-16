"""Frozen 0.2 parsing for legacy ``main_script.py`` cell markers."""

from __future__ import annotations

import hashlib
import io
import re
import tokenize
from dataclasses import dataclass
from pathlib import Path
from uuid import UUID

from .errors import ScriptError

DIRECT_SCRIPT_NAME = "main_script.py"
DIRECT_SCRIPT_RELATIVE = Path(".tactus") / DIRECT_SCRIPT_NAME
MAX_SCRIPT_BYTES = 1024 * 1024
MAX_SCRIPT_CELLS = 1024

_CELL_MARKER = re.compile(r"^#[ \t]*%%(?P<meta>.*)$")
_MARKDOWN_META = re.compile(r"^\s*\[markdown\]\s*", re.IGNORECASE)
_STABLE_META = re.compile(
    r"^\[tactus-cell:(?P<cell_id>[0-9a-fA-F-]{36})\](?:\s+(?P<title>.*))?$"
)


@dataclass(frozen=True)
class DirectScriptCell:
    """One independently compilable cell in source order."""

    ordinal: int
    title: str
    source: str
    start_line: int
    end_line: int
    kind: str
    digest: str
    cell_id: UUID | None = None

    @property
    def executable(self) -> bool:
        """Return whether this cell contains code for daemon submission."""
        return self.kind == "code" and bool(self.source.strip())


@dataclass(frozen=True)
class DirectScript:
    """A bounded script parsed without executing or persisting state."""

    path: Path
    content: str
    digest: str
    cells: tuple[DirectScriptCell, ...]

    @property
    def executable_cells(self) -> tuple[DirectScriptCell, ...]:
        """Return executable cells in stable source order."""
        return tuple(cell for cell in self.cells if cell.executable)


def load_direct_script(root: Path) -> DirectScript:
    """Read and parse the bounded ``.tactus/main_script.py`` projection."""
    path = root.expanduser().resolve() / DIRECT_SCRIPT_RELATIVE
    try:
        with path.open("rb") as stream:
            content_bytes = stream.read(MAX_SCRIPT_BYTES + 1)
    except OSError as exc:
        raise ScriptError(f"cannot read {path}: {exc}") from exc
    if len(content_bytes) > MAX_SCRIPT_BYTES:
        raise ScriptError(f"{path.name} exceeds {MAX_SCRIPT_BYTES} bytes")
    try:
        content = content_bytes.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ScriptError(f"{path.name} must be UTF-8") from exc
    return parse_direct_script(content, path=path)


def parse_direct_script(content: str, *, path: Path) -> DirectScript:
    """Parse and compile a bounded ``# %%`` script without executing it."""
    if not isinstance(content, str):
        raise TypeError("direct main script content must be text")
    encoded = content.encode("utf-8")
    if len(encoded) > MAX_SCRIPT_BYTES:
        raise ScriptError(f"{path.name} exceeds {MAX_SCRIPT_BYTES} bytes")
    resolved = path.expanduser().resolve()
    _compile(content, resolved, cell=None)

    lines = content.splitlines(keepends=True)
    markers = _tokenized_markers(content)
    if len(markers) > MAX_SCRIPT_CELLS:
        raise ScriptError(f"{resolved.name} exceeds {MAX_SCRIPT_CELLS} cells")
    raw_cells: list[tuple[str, str, int, int, str, UUID | None]] = []
    if not markers:
        raw_cells.append(("Script", content, 1, max(1, len(lines)), "code", None))
    else:
        first_marker = markers[0][0]
        preamble = "".join(lines[:first_marker])
        if preamble.strip():
            raw_cells.append(
                ("Preamble", preamble, 1, max(1, first_marker), "code", None)
            )
        for marker_index, (line_index, metadata) in enumerate(markers):
            next_index = (
                markers[marker_index + 1][0]
                if marker_index + 1 < len(markers)
                else len(lines)
            )
            kind, title, cell_id = _cell_metadata(metadata, marker_index + 1)
            raw_cells.append(
                (
                    title,
                    "".join(lines[line_index + 1 : next_index]),
                    line_index + 2,
                    max(line_index + 2, next_index),
                    kind,
                    cell_id,
                )
            )
    if len(raw_cells) > MAX_SCRIPT_CELLS:
        raise ScriptError(f"{resolved.name} exceeds {MAX_SCRIPT_CELLS} cells")

    cells: list[DirectScriptCell] = []
    stable_ids: set[UUID] = set()
    for ordinal, raw in enumerate(raw_cells, start=1):
        title, source, start_line, end_line, kind, cell_id = raw
        if cell_id is not None:
            if cell_id in stable_ids:
                raise ScriptError(f"duplicate stable cell id: {cell_id}")
            stable_ids.add(cell_id)
        if kind == "code" and source.strip():
            padded = "\n" * max(0, start_line - 1) + source
            _compile(padded, resolved, cell=(ordinal, title))
        cells.append(
            DirectScriptCell(
                ordinal=ordinal,
                title=title,
                source=source,
                start_line=start_line,
                end_line=end_line,
                kind=kind,
                digest=_source_digest(source),
                cell_id=cell_id,
            )
        )
    return DirectScript(
        path=resolved,
        content=content,
        digest=hashlib.sha256(encoded).hexdigest(),
        cells=tuple(cells),
    )


def _tokenized_markers(content: str) -> list[tuple[int, str]]:
    markers: list[tuple[int, str]] = []
    try:
        for token in tokenize.generate_tokens(io.StringIO(content).readline):
            if token.type != tokenize.COMMENT or token.start[1] != 0:
                continue
            match = _CELL_MARKER.fullmatch(token.string)
            if match is not None:
                markers.append((token.start[0] - 1, match.group("meta").strip()))
    except (IndentationError, tokenize.TokenError) as exc:
        raise ScriptError(f"cannot tokenize {DIRECT_SCRIPT_NAME}: {exc}") from exc
    return markers


def _cell_metadata(
    metadata: str,
    fallback_ordinal: int,
) -> tuple[str, str, UUID | None]:
    if _MARKDOWN_META.match(metadata):
        title = _MARKDOWN_META.sub("", metadata, count=1).strip()
        return "markdown", title or f"Notes {fallback_ordinal}", None
    stable = _STABLE_META.fullmatch(metadata)
    if stable is not None:
        try:
            cell_id = UUID(stable.group("cell_id"))
        except ValueError as exc:
            raise ScriptError("stable cell marker contains an invalid UUID") from exc
        title = (stable.group("title") or "").strip()
        if len(title) > 200:
            raise ScriptError("cell title exceeds 200 characters")
        return "code", title or f"Cell {fallback_ordinal}", cell_id
    title = metadata.strip()
    if len(title) > 200:
        raise ScriptError("cell title exceeds 200 characters")
    return "code", title or f"Cell {fallback_ordinal}", None


def _compile(
    source: str,
    path: Path,
    *,
    cell: tuple[int, str] | None,
) -> None:
    try:
        compile(source, str(path), "exec")
    except SyntaxError as exc:
        line = "?" if exc.lineno is None else str(exc.lineno)
        column = "?" if exc.offset is None else str(exc.offset)
        context = "" if cell is None else f" cell {cell[0]} ({cell[1]}):"
        raise ScriptError(
            f"{path.name}:{line}:{column}:{context} "
            f"{exc.msg or 'invalid Python syntax'}"
        ) from exc


def _source_digest(source: str) -> str:
    normalized = source.rstrip() + "\n"
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


__all__ = [
    "DIRECT_SCRIPT_NAME",
    "DIRECT_SCRIPT_RELATIVE",
    "MAX_SCRIPT_BYTES",
    "MAX_SCRIPT_CELLS",
    "DirectScript",
    "DirectScriptCell",
    "load_direct_script",
    "parse_direct_script",
]
