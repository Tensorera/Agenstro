"""Strict task-package manifest models for offline authoring."""

from __future__ import annotations

import json
import re
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

MANIFEST_NAME = "segno-flow.json"
MAX_MANIFEST_BYTES = 1024 * 1024
_TASK_ID = re.compile(r"[a-z](?:[a-z0-9]|-(?!-))*[a-z0-9]\Z|[a-z]\Z")
_TIMEZONE = re.compile(r"[A-Za-z0-9/_+\-]+\Z")
_WINDOWS_DEVICE_NAMES = {
    "CON",
    "PRN",
    "AUX",
    "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}


class ManifestError(ValueError):
    """A manifest is not valid Segno package metadata."""


def _has_control(value: str) -> bool:
    return any(unicodedata.category(character).startswith("C") for character in value)


@dataclass(frozen=True, slots=True)
class MisfirePolicy:
    """Explicit bounded downtime behavior."""

    kind: str
    grace_seconds: int | None = None
    limit: int | None = None


@dataclass(frozen=True, slots=True)
class OverlapPolicy:
    """Explicit same-task concurrency behavior."""

    kind: str
    limit: int | None = None


@dataclass(frozen=True, slots=True)
class RetryPolicy:
    """Explicit scheduler dispatch retry behavior."""

    kind: str
    max_attempts: int | None = None
    delay_seconds: int | None = None


@dataclass(frozen=True, slots=True)
class ScheduleManifest:
    """Authoring representation of the Rust-owned schedule policy."""

    cron_dialect: str
    cron: str
    timezone: str
    dst_gap: str
    dst_fold: str
    misfire: MisfirePolicy
    overlap: OverlapPolicy
    retry: RetryPolicy
    jitter_seconds: int


@dataclass(frozen=True, slots=True)
class ScriptsManifest:
    """Three immutable stage paths delegated to Clef and Tactus."""

    pre: str
    main: str
    post: str


@dataclass(frozen=True, slots=True)
class PackageManifest:
    """Strict schema-version-one task-package manifest."""

    schema_version: int
    id: str
    name: str
    schedule: ScheduleManifest
    scripts: ScriptsManifest

    def to_dict(self) -> dict[str, object]:
        """Return the canonical JSON-compatible manifest shape."""

        misfire: dict[str, object] = {"kind": self.schedule.misfire.kind}
        if self.schedule.misfire.grace_seconds is not None:
            misfire["grace_seconds"] = self.schedule.misfire.grace_seconds
        if self.schedule.misfire.limit is not None:
            misfire["limit"] = self.schedule.misfire.limit

        overlap: dict[str, object] = {"kind": self.schedule.overlap.kind}
        if self.schedule.overlap.limit is not None:
            overlap["limit"] = self.schedule.overlap.limit

        retry: dict[str, object] = {"kind": self.schedule.retry.kind}
        if self.schedule.retry.max_attempts is not None:
            retry["max_attempts"] = self.schedule.retry.max_attempts
        if self.schedule.retry.delay_seconds is not None:
            retry["delay_seconds"] = self.schedule.retry.delay_seconds

        return {
            "schema_version": self.schema_version,
            "id": self.id,
            "name": self.name,
            "schedule": {
                "cron_dialect": self.schedule.cron_dialect,
                "cron": self.schedule.cron,
                "timezone": self.schedule.timezone,
                "dst_gap": self.schedule.dst_gap,
                "dst_fold": self.schedule.dst_fold,
                "misfire": misfire,
                "overlap": overlap,
                "retry": retry,
                "jitter_seconds": self.schedule.jitter_seconds,
            },
            "scripts": {
                "pre": self.scripts.pre,
                "main": self.scripts.main,
                "post": self.scripts.post,
            },
        }


def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ManifestError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _object(value: object, location: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise ManifestError(f"{location} must be an object")
    return value


def _exact_keys(value: Mapping[str, object], expected: set[str], location: str) -> None:
    missing = expected - value.keys()
    unknown = value.keys() - expected
    if missing:
        raise ManifestError(f"{location} is missing: {', '.join(sorted(missing))}")
    if unknown:
        raise ManifestError(f"{location} contains unknown fields: {', '.join(sorted(unknown))}")


def _string(value: object, location: str, *, max_bytes: int) -> str:
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > max_bytes:
        raise ManifestError(f"{location} must be a non-empty string of at most {max_bytes} bytes")
    return value


def _integer(value: object, location: str, *, minimum: int, maximum: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise ManifestError(f"{location} must be an integer from {minimum} through {maximum}")
    return value


def _enum(value: object, location: str, choices: set[str]) -> str:
    if not isinstance(value, str) or value not in choices:
        raise ManifestError(f"{location} must be one of: {', '.join(sorted(choices))}")
    return value


def _parse_misfire(value: object) -> MisfirePolicy:
    raw = _object(value, "schedule.misfire")
    kind = _enum(
        raw.get("kind"),
        "schedule.misfire.kind",
        {"skip", "coalesce", "bounded_catch_up"},
    )
    expected = {
        "skip": {"kind", "grace_seconds"},
        "coalesce": {"kind"},
        "bounded_catch_up": {"kind", "limit"},
    }[kind]
    _exact_keys(raw, expected, "schedule.misfire")
    if kind == "skip":
        return MisfirePolicy(
            kind=kind,
            grace_seconds=_integer(
                raw["grace_seconds"],
                "schedule.misfire.grace_seconds",
                minimum=0,
                maximum=86_400,
            ),
        )
    if kind == "bounded_catch_up":
        return MisfirePolicy(
            kind=kind,
            limit=_integer(raw["limit"], "schedule.misfire.limit", minimum=1, maximum=1_000),
        )
    return MisfirePolicy(kind=kind)


def _parse_overlap(value: object) -> OverlapPolicy:
    raw = _object(value, "schedule.overlap")
    kind = _enum(
        raw.get("kind"),
        "schedule.overlap.kind",
        {"forbid", "queue_one", "allow_with_limit"},
    )
    expected = {"kind", "limit"} if kind == "allow_with_limit" else {"kind"}
    _exact_keys(raw, expected, "schedule.overlap")
    limit = (
        _integer(raw["limit"], "schedule.overlap.limit", minimum=1, maximum=64)
        if kind == "allow_with_limit"
        else None
    )
    return OverlapPolicy(kind=kind, limit=limit)


def _parse_retry(value: object) -> RetryPolicy:
    raw = _object(value, "schedule.retry")
    kind = _enum(raw.get("kind"), "schedule.retry.kind", {"none", "bounded_idempotent"})
    expected = (
        {"kind", "max_attempts", "delay_seconds"} if kind == "bounded_idempotent" else {"kind"}
    )
    _exact_keys(raw, expected, "schedule.retry")
    if kind == "bounded_idempotent":
        return RetryPolicy(
            kind=kind,
            max_attempts=_integer(
                raw["max_attempts"],
                "schedule.retry.max_attempts",
                minimum=1,
                maximum=32,
            ),
            delay_seconds=_integer(
                raw["delay_seconds"],
                "schedule.retry.delay_seconds",
                minimum=0,
                maximum=86_400,
            ),
        )
    return RetryPolicy(kind=kind)


def _portable_script_path(value: object, location: str) -> str:
    path = _string(value, location, max_bytes=4_096)
    if path.startswith("/") or "\\" in path or "\0" in path or _has_control(path):
        raise ManifestError(f"{location} must be a portable relative path")
    components = path.split("/")
    if len(components) > 32 or any(
        not component
        or component in {".", ".."}
        or len(component.encode("utf-8")) > 255
        or ":" in component
        or component.endswith((".", " "))
        or component.split(".", 1)[0].upper() in _WINDOWS_DEVICE_NAMES
        for component in components
    ):
        raise ManifestError(f"{location} must be a portable relative path")
    if not path.endswith(".py"):
        raise ManifestError(f"{location} must reference a .py file")
    return path


def parse_manifest(payload: bytes) -> PackageManifest:
    """Parse bounded UTF-8 JSON and reject duplicate or unknown fields."""

    if len(payload) > MAX_MANIFEST_BYTES:
        raise ManifestError("manifest exceeds the 1 MiB limit")
    try:
        raw_value = json.loads(payload.decode("utf-8"), object_pairs_hook=_reject_duplicate_keys)
    except ManifestError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ManifestError("manifest must be valid UTF-8 JSON") from error

    raw = _object(raw_value, "manifest")
    _exact_keys(raw, {"schema_version", "id", "name", "schedule", "scripts"}, "manifest")
    schema_version = _integer(raw["schema_version"], "schema_version", minimum=1, maximum=1)
    task_id = _string(raw["id"], "id", max_bytes=64)
    if _TASK_ID.fullmatch(task_id) is None:
        raise ManifestError("id must be a lower-case dash-separated identifier")
    name = _string(raw["name"], "name", max_bytes=120)

    schedule_raw = _object(raw["schedule"], "schedule")
    _exact_keys(
        schedule_raw,
        {
            "cron_dialect",
            "cron",
            "timezone",
            "dst_gap",
            "dst_fold",
            "misfire",
            "overlap",
            "retry",
            "jitter_seconds",
        },
        "schedule",
    )
    cron = _string(schedule_raw["cron"], "schedule.cron", max_bytes=200)
    if (
        _has_control(cron)
        or any(character.isspace() and character != " " for character in cron)
        or len([field for field in cron.split(" ") if field]) != 5
    ):
        raise ManifestError("schedule.cron must contain five bounded fields")
    timezone = _string(schedule_raw["timezone"], "schedule.timezone", max_bytes=100)
    if timezone.lower() == "local" or _TIMEZONE.fullmatch(timezone) is None:
        raise ManifestError("schedule.timezone must be an explicit IANA name")
    schedule = ScheduleManifest(
        cron_dialect=_enum(schedule_raw["cron_dialect"], "schedule.cron_dialect", {"unix5"}),
        cron=cron,
        timezone=timezone,
        dst_gap=_enum(schedule_raw["dst_gap"], "schedule.dst_gap", {"skip", "next_valid"}),
        dst_fold=_enum(
            schedule_raw["dst_fold"],
            "schedule.dst_fold",
            {"first", "second", "both"},
        ),
        misfire=_parse_misfire(schedule_raw["misfire"]),
        overlap=_parse_overlap(schedule_raw["overlap"]),
        retry=_parse_retry(schedule_raw["retry"]),
        jitter_seconds=_integer(
            schedule_raw["jitter_seconds"],
            "schedule.jitter_seconds",
            minimum=0,
            maximum=86_400,
        ),
    )

    scripts_raw = _object(raw["scripts"], "scripts")
    _exact_keys(scripts_raw, {"pre", "main", "post"}, "scripts")
    scripts = ScriptsManifest(
        pre=_portable_script_path(scripts_raw["pre"], "scripts.pre"),
        main=_portable_script_path(scripts_raw["main"], "scripts.main"),
        post=_portable_script_path(scripts_raw["post"], "scripts.post"),
    )
    collision_keys = {
        unicodedata.normalize("NFC", value).lower()
        for value in (scripts.pre, scripts.main, scripts.post)
    }
    if len(collision_keys) != 3:
        raise ManifestError("pre, main, and post must reference different files")
    return PackageManifest(schema_version, task_id, name, schedule, scripts)


def load_manifest(path: Path) -> PackageManifest:
    """Read a manifest with an explicit byte limit."""

    with path.open("rb") as source:
        payload = source.read(MAX_MANIFEST_BYTES + 1)
    return parse_manifest(payload)
