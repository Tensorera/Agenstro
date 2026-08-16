"""Bounded readers for durable Segno import and migration evidence."""

from __future__ import annotations

import json
import re
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from segno_flow.client import ImportResult

MAX_REPORT_BYTES = 1024 * 1024
MAX_MIGRATION_IMPORTS = 1_000
MAX_MIGRATION_WARNINGS = 1_000
_DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


class ReportError(ValueError):
    """A report is oversized, ambiguous, or does not match its schema."""


@dataclass(frozen=True, slots=True)
class MigrationReport:
    """Bounded migration summary containing daemon import results."""

    schema_version: int
    source: str
    imports: tuple[ImportResult, ...]
    warnings: tuple[str, ...]


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ReportError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _load(path: Path) -> Mapping[str, object]:
    with path.open("rb") as source:
        payload = source.read(MAX_REPORT_BYTES + 1)
    if len(payload) > MAX_REPORT_BYTES:
        raise ReportError("report exceeds the 1 MiB limit")
    try:
        value = json.loads(payload.decode("utf-8"), object_pairs_hook=_reject_duplicate_keys)
    except ReportError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReportError("report must be valid UTF-8 JSON") from error
    if not isinstance(value, dict):
        raise ReportError("report root must be an object")
    return value


def _exact_keys(value: Mapping[str, object], expected: set[str], location: str) -> None:
    missing = expected - value.keys()
    unknown = value.keys() - expected
    if missing or unknown:
        raise ReportError(f"{location} fields do not match schema")


def _import_result(value: object) -> ImportResult:
    if not isinstance(value, dict):
        raise ReportError("import report must be an object")
    _exact_keys(
        value,
        {"task_id", "revision", "package_digest", "workflow_spec_digest", "enabled"},
        "import report",
    )
    task_id = value["task_id"]
    revision = value["revision"]
    package_digest = value["package_digest"]
    workflow_spec_digest = value["workflow_spec_digest"]
    enabled = value["enabled"]
    if (
        not isinstance(task_id, str)
        or not task_id
        or isinstance(revision, bool)
        or not isinstance(revision, int)
        or revision < 1
        or not isinstance(package_digest, str)
        or _DIGEST.fullmatch(package_digest) is None
        or not isinstance(workflow_spec_digest, str)
        or _DIGEST.fullmatch(workflow_spec_digest) is None
        or not isinstance(enabled, bool)
    ):
        raise ReportError("import report contains invalid values")
    return ImportResult(task_id, revision, package_digest, workflow_spec_digest, enabled)


def read_import_report(path: Path) -> ImportResult:
    """Read the strict JSON result emitted by a package import."""

    return _import_result(_load(path))


def read_migration_report(path: Path) -> MigrationReport:
    """Read a bounded migration envelope without opening referenced packages."""

    raw = _load(path)
    _exact_keys(raw, {"schema_version", "source", "imports", "warnings"}, "migration report")
    version = raw["schema_version"]
    source = raw["source"]
    imports = raw["imports"]
    warnings = raw["warnings"]
    if version != 1 or not isinstance(source, str) or not source:
        raise ReportError("migration report header is invalid")
    if not isinstance(imports, list) or len(imports) > MAX_MIGRATION_IMPORTS:
        raise ReportError("migration report import count exceeds its limit")
    if (
        not isinstance(warnings, list)
        or len(warnings) > MAX_MIGRATION_WARNINGS
        or any(not isinstance(warning, str) for warning in warnings)
    ):
        raise ReportError("migration report warnings are invalid")
    return MigrationReport(
        schema_version=version,
        source=source,
        imports=tuple(_import_result(item) for item in imports),
        warnings=tuple(warnings),
    )
