"""Pure normalized adapter-protocol transcript conformance."""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from enum import Enum
from typing import cast

from .manifest import AdapterManifest

_MAX_FRAMES = 4_096
_MAX_FRAME_BYTES = 256 * 1_024
_MAX_EVENT_PAYLOAD_BYTES = 64 * 1_024


class TranscriptError(ValueError):
    """A fake transcript violates the normalized adapter protocol."""


class AgentEventKind(str, Enum):
    """Provider-neutral events accepted by the Rust agent port."""

    SESSION_STARTED = "session_started"
    CONTENT_DELTA = "content_delta"
    TOOL_STARTED = "tool_started"
    TOOL_COMPLETED = "tool_completed"
    APPROVAL_REQUESTED = "approval_requested"
    APPROVAL_RESOLVED = "approval_resolved"
    FILE_CHANGE_REPORTED = "file_change_reported"
    USAGE_UPDATED = "usage_updated"
    DIAGNOSTIC = "diagnostic"
    TURN_COMPLETED = "turn_completed"
    TURN_FAILED = "turn_failed"


_TERMINAL = frozenset({AgentEventKind.TURN_COMPLETED, AgentEventKind.TURN_FAILED})


def _mapping(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise TranscriptError(f"{field_name} must be an object")
    untyped = cast(Mapping[object, object], value)
    if not all(isinstance(key, str) for key in untyped):
        raise TranscriptError(f"{field_name} keys must be strings")
    return cast(Mapping[str, object], untyped)


def _sequence(value: object, field_name: str) -> Sequence[object]:
    if isinstance(value, str | bytes) or not isinstance(value, Sequence):
        raise TranscriptError(f"{field_name} must be an array")
    return cast(Sequence[object], value)


def _string(value: object, field_name: str, *, maximum: int = 512) -> str:
    if not isinstance(value, str) or not value.strip():
        raise TranscriptError(f"{field_name} must be a non-empty string")
    if len(value) > maximum or any(
        ord(character) < 32 and character not in "\n\r\t" for character in value
    ):
        raise TranscriptError(
            f"{field_name} is too long or contains control characters"
        )
    return value


def _integer(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise TranscriptError(f"{field_name} must be an integer >= 0")
    return value


def _bounded_frames(
    frames: Iterable[Mapping[str, object]],
) -> tuple[Mapping[str, object], ...]:
    result: list[Mapping[str, object]] = []
    for frame in frames:
        if len(result) >= _MAX_FRAMES:
            raise TranscriptError(f"transcript exceeds {_MAX_FRAMES} frames")
        mapping = _mapping(frame, f"frames[{len(result)}]")
        encoded = json.dumps(
            mapping,
            ensure_ascii=False,
            allow_nan=False,
            separators=(",", ":"),
        ).encode("utf-8")
        if len(encoded) > _MAX_FRAME_BYTES:
            raise TranscriptError(
                f"frames[{len(result)}] exceeds {_MAX_FRAME_BYTES} bytes"
            )
        result.append(mapping)
    if not result:
        raise TranscriptError("transcript must contain an initialized frame")
    return tuple(result)


@dataclass(frozen=True, slots=True)
class NormalizedEvent:
    """One validated provider-neutral fake event."""

    kind: AgentEventKind
    session_id: str
    turn_id: str
    sequence: int
    occurred_at: datetime
    provider: str
    payload_bytes: int
    message: str | None = None

    @property
    def terminal(self) -> bool:
        """Return whether this is the unique terminal event."""
        return self.kind in _TERMINAL


@dataclass(frozen=True, slots=True)
class ConformanceResult:
    """Auditable result of one offline transcript contract check."""

    adapter_id: str
    provider: str
    provider_version: str
    capabilities: frozenset[str]
    event_count: int
    terminal_event: AgentEventKind


def _initialize(
    manifest: AdapterManifest, frame: Mapping[str, object]
) -> tuple[str, frozenset[str]]:
    expected = {
        "type",
        "adapter_id",
        "provider",
        "protocol_major",
        "protocol_minor",
        "provider_version",
        "capabilities",
        "auth_methods",
    }
    if set(frame) != expected:
        raise TranscriptError("initialized frame has missing or unknown fields")
    if frame["type"] != "initialized":
        raise TranscriptError("the first frame must be initialized")
    if frame["adapter_id"] != manifest.id or frame["provider"] != manifest.provider:
        raise TranscriptError("initialized adapter/provider does not match manifest")
    if _integer(frame["protocol_major"], "protocol_major") != manifest.protocol_major:
        raise TranscriptError("adapter protocol major mismatch")
    if _integer(frame["protocol_minor"], "protocol_minor") < manifest.protocol_minor:
        raise TranscriptError("adapter protocol minor is older than the manifest")
    provider_version = _string(frame["provider_version"], "provider_version")
    capabilities = frozenset(
        _string(value, f"capabilities[{index}]")
        for index, value in enumerate(_sequence(frame["capabilities"], "capabilities"))
    )
    if len(capabilities) > 64:
        raise TranscriptError("initialized capabilities exceed 64 items")
    if not capabilities <= manifest.static_capabilities:
        raise TranscriptError("dynamic capabilities exceed the manifest declaration")
    auth_methods = tuple(
        _string(value, f"auth_methods[{index}]")
        for index, value in enumerate(_sequence(frame["auth_methods"], "auth_methods"))
    )
    if len(auth_methods) > 16:
        raise TranscriptError("initialized auth methods exceed 16 items")
    return provider_version, capabilities


def _event(
    manifest: AdapterManifest,
    frame: Mapping[str, object],
    *,
    expected_sequence: int,
) -> NormalizedEvent:
    allowed = {
        "type",
        "schema_version",
        "event",
        "session_id",
        "turn_id",
        "sequence",
        "occurred_at",
        "provider",
        "payload_bytes",
        "message",
    }
    required = allowed - {"message"}
    if not required <= set(frame) or set(frame) - allowed:
        raise TranscriptError("event frame has missing or unknown fields")
    if frame["type"] != "event" or frame["schema_version"] != "1.0":
        raise TranscriptError("event frame type or schema version is unsupported")
    if frame["provider"] != manifest.provider:
        raise TranscriptError("event provider does not match manifest")
    sequence = _integer(frame["sequence"], "sequence")
    if sequence != expected_sequence:
        raise TranscriptError(
            f"event sequence {sequence} does not equal {expected_sequence}"
        )
    payload_bytes = _integer(frame["payload_bytes"], "payload_bytes")
    if payload_bytes > _MAX_EVENT_PAYLOAD_BYTES:
        raise TranscriptError("event payload exceeds the normalized protocol limit")
    try:
        kind = AgentEventKind(_string(frame["event"], "event"))
    except ValueError as error:
        raise TranscriptError("unknown normalized event kind") from error
    timestamp = _string(frame["occurred_at"], "occurred_at")
    try:
        occurred_at = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError as error:
        raise TranscriptError("occurred_at is not ISO-8601") from error
    if occurred_at.tzinfo is None:
        raise TranscriptError("occurred_at must include a timezone")
    raw_message = frame.get("message")
    message = (
        None if raw_message is None else _string(raw_message, "message", maximum=4096)
    )
    return NormalizedEvent(
        kind=kind,
        session_id=_string(frame["session_id"], "session_id"),
        turn_id=_string(frame["turn_id"], "turn_id"),
        sequence=sequence,
        occurred_at=occurred_at,
        provider=manifest.provider,
        payload_bytes=payload_bytes,
        message=message,
    )


def check_transcript(
    manifest: AdapterManifest,
    frames: Iterable[Mapping[str, object]],
) -> ConformanceResult:
    """Validate one bounded fake transcript without launching an adapter."""
    values = _bounded_frames(frames)
    provider_version, capabilities = _initialize(manifest, values[0])
    events: list[NormalizedEvent] = []
    terminal = False
    correlation: tuple[str, str] | None = None
    for sequence, frame in enumerate(values[1:], start=1):
        event = _event(manifest, frame, expected_sequence=sequence)
        if sequence == 1 and event.kind is not AgentEventKind.SESSION_STARTED:
            raise TranscriptError("the first normalized event must start the session")
        if terminal:
            raise TranscriptError("event appeared after the terminal event")
        current_correlation = (event.session_id, event.turn_id)
        if correlation is None:
            correlation = current_correlation
        elif current_correlation != correlation:
            raise TranscriptError("session/turn correlation changed within one turn")
        terminal = event.terminal
        events.append(event)
    if not events or not terminal:
        raise TranscriptError("transcript must contain exactly one terminal turn event")
    return ConformanceResult(
        adapter_id=manifest.id,
        provider=manifest.provider,
        provider_version=provider_version,
        capabilities=capabilities,
        event_count=len(events),
        terminal_event=events[-1].kind,
    )


__all__ = [
    "AgentEventKind",
    "ConformanceResult",
    "NormalizedEvent",
    "TranscriptError",
    "check_transcript",
]
