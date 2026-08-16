"""Small, language-neutral JSON/JSONL boundary for Tactus plugins.

Each plugin process reads exactly one JSON request from stdin, may write zero or
more JSONL events, and writes exactly one terminal ``result`` record.  Human
diagnostics belong on stderr and are deliberately not part of the protocol.
"""

from __future__ import annotations

import json
import math
import traceback
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from io import TextIOWrapper
from typing import TextIO, cast

API_VERSION = "agenstro.plugin/v1"

type RequestId = str | int
type JsonObject = dict[str, object]


@dataclass(frozen=True, slots=True)
class PluginRequest:
    """A validated request delivered to one reference plugin."""

    id: RequestId
    method: str
    params: JsonObject


class PluginError(Exception):
    """An expected plugin failure that is safe to expose to the caller."""

    def __init__(
        self,
        code: str,
        message: str,
        *,
        details: Mapping[str, object] | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = dict(details or {})

    def as_json(self) -> JsonObject:
        """Return the stable wire representation of this error."""
        value: JsonObject = {"code": self.code, "message": self.message}
        if self.details:
            value["details"] = self.details
        return value


class EventWriter:
    """Write non-terminal events for one request."""

    def __init__(self, request_id: RequestId | None, stream: TextIO) -> None:
        self.request_id = request_id
        self._stream = stream
        self._terminal_written = False

    def event(self, event_type: str, **payload: object) -> None:
        """Write one non-terminal event line."""
        if not event_type:
            raise ValueError("event type must be non-empty")
        if self._terminal_written:
            raise RuntimeError("cannot write an event after the terminal result")
        if "type" in payload:
            raise ValueError("event payload cannot replace its subtype")
        self._write(
            {
                "type": "event",
                "id": self.request_id,
                "event": {"type": event_type, **payload},
            }
        )

    def success(self, value: object) -> None:
        """Write the unique successful terminal result."""
        self._terminal(
            {"type": "result", "id": self.request_id, "ok": True, "value": value}
        )

    def failure(self, error: Mapping[str, object]) -> None:
        """Write the unique failed terminal result."""
        self._terminal(
            {"type": "result", "id": self.request_id, "ok": False, "error": dict(error)}
        )

    def _terminal(self, value: JsonObject) -> None:
        if self._terminal_written:
            raise RuntimeError("terminal result was already written")
        self._write(value)
        self._terminal_written = True

    def _write(self, value: object) -> None:
        encoded = json.dumps(
            value,
            allow_nan=False,
            ensure_ascii=False,
            separators=(",", ":"),
        )
        self._stream.write(encoded + "\n")
        self._stream.flush()


type PluginHandler = Callable[[PluginRequest, EventWriter], object]


def configure_utf8_standard_stream(stream: TextIO) -> TextIO:
    """Make a CPython standard stream use the plugin protocol's UTF-8 encoding.

    ``TextIOWrapper.reconfigure`` preserves the wrapper and its underlying file
    descriptor, so callers retain ownership of the standard stream.  In-memory
    streams such as ``StringIO`` are deliberately returned unchanged.
    """
    if isinstance(stream, TextIOWrapper):
        stream.reconfigure(encoding="utf-8", errors="strict")
    return cast(TextIO, stream)


def split_jsonl(value: str) -> list[str]:
    """Split only on the JSONL LF delimiter, never on Unicode line characters."""
    lines = value.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    return [line[:-1] if line.endswith("\r") else line for line in lines]


def parse_request(value: object) -> PluginRequest:
    """Validate an already decoded request object."""
    request = _string_keyed_object(value)
    if request is None:
        raise PluginError("invalid_request", "request must be a JSON object")
    if request.get("api") != API_VERSION:
        raise PluginError(
            "unsupported_api",
            f"request api must be {API_VERSION!r}",
            details={"received": request.get("api")},
        )
    request_id = request.get("id")
    if isinstance(request_id, bool) or not isinstance(request_id, (str, int)):
        raise PluginError("invalid_request", "request id must be a string or integer")
    method = request.get("method")
    if not isinstance(method, str) or not method:
        raise PluginError(
            "invalid_request", "request method must be a non-empty string"
        )
    params = _string_keyed_object(request.get("params"))
    if params is None:
        raise PluginError("invalid_request", "request params must be a JSON object")
    return PluginRequest(id=request_id, method=method, params=params)


def run_plugin(
    handler: PluginHandler,
    *,
    stdin: TextIO,
    stdout: TextIO,
    stderr: TextIO,
) -> int:
    """Run one request through ``handler`` and emit exactly one terminal line."""
    decoded: object = None
    request_id: RequestId | None = None
    try:
        decoded = json.load(
            stdin,
            object_pairs_hook=_unique_object,
            parse_constant=_reject_json_constant,
            parse_float=_parse_finite_float,
        )
        request_id = _candidate_id(decoded)
        request = parse_request(decoded)
    except (json.JSONDecodeError, ValueError) as exc:
        writer = EventWriter(request_id, stdout)
        details: dict[str, object] = {}
        if isinstance(exc, json.JSONDecodeError):
            details = {"line": exc.lineno, "column": exc.colno}
        error = PluginError(
            "invalid_json",
            "stdin must contain exactly one JSON request",
            details=details,
        )
        print(f"plugin protocol: {error.message}: {exc}", file=stderr)
        writer.failure(error.as_json())
        return 2
    except PluginError as exc:
        writer = EventWriter(request_id, stdout)
        print(f"plugin protocol: {exc.message}", file=stderr)
        writer.failure(exc.as_json())
        return 2

    writer = EventWriter(request.id, stdout)
    try:
        value = handler(request, writer)
        writer.success(value)
    except PluginError as exc:
        print(f"plugin: {exc.code}: {exc.message}", file=stderr)
        try:
            writer.failure(exc.as_json())
        except (TypeError, ValueError):
            writer.failure(
                PluginError(
                    "internal_error",
                    "plugin error details were not valid JSON",
                ).as_json()
            )
        return 1
    except KeyboardInterrupt:
        error = PluginError("interrupted", "plugin invocation was interrupted")
        print("plugin: interrupted", file=stderr)
        writer.failure(error.as_json())
        return 130
    except Exception as exc:
        print(f"plugin: unexpected {type(exc).__name__}: {exc}", file=stderr)
        traceback.print_exc(file=stderr)
        writer.failure(
            PluginError(
                "internal_error",
                "plugin failed unexpectedly",
                details={"exception": type(exc).__name__},
            ).as_json()
        )
        return 1

    return 0


def _candidate_id(value: object) -> RequestId | None:
    request = _string_keyed_object(value)
    if request is None:
        return None
    candidate = request.get("id")
    if isinstance(candidate, bool) or not isinstance(candidate, (str, int)):
        return None
    return candidate


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate key {key!r}")
        value[key] = item
    return value


def _reject_json_constant(value: str) -> object:
    raise ValueError(f"non-finite JSON number {value}")


def _parse_finite_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise ValueError(f"JSON number is outside the finite float range: {value}")
    try:
        underflowed = parsed == 0.0 and Decimal(value) != 0
    except InvalidOperation as exc:
        raise ValueError(f"invalid JSON number: {value}") from exc
    if underflowed:
        raise ValueError(f"JSON number is outside the finite float range: {value}")
    return parsed


def _string_keyed_object(value: object) -> dict[str, object] | None:
    if not isinstance(value, dict):
        return None
    raw = cast(dict[object, object], value)
    converted: dict[str, object] = {}
    for key, item in raw.items():
        if not isinstance(key, str):
            return None
        converted[key] = item
    return converted


__all__ = [
    "API_VERSION",
    "EventWriter",
    "JsonObject",
    "PluginError",
    "PluginHandler",
    "PluginRequest",
    "RequestId",
    "configure_utf8_standard_stream",
    "parse_request",
    "run_plugin",
    "split_jsonl",
]
