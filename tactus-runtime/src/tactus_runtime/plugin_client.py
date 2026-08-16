"""Language-neutral one-shot JSONL client for provider and effect plugins."""

from __future__ import annotations

import json
import math
import os
import shutil
import subprocess
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import cast
from uuid import uuid4

from .errors import PluginProtocolError, ToolError
from .plugin_protocol import split_jsonl
from .workspace import PLUGIN_API, JsonValue

PluginExecutor = Callable[..., subprocess.CompletedProcess[str]]


@dataclass(frozen=True, slots=True)
class PluginResponse:
    """Validated terminal response from a one-shot plugin process."""

    request_id: str
    ok: bool
    value: JsonValue | None
    error: JsonValue | None
    events: tuple[dict[str, JsonValue], ...]
    exit_code: int


def invoke_plugin(
    command: Sequence[str],
    *,
    method: str,
    params: Mapping[str, JsonValue],
    cwd: Path,
    environment: Mapping[str, str],
    executor: PluginExecutor = subprocess.run,
) -> PluginResponse:
    """Invoke one plugin and require exactly one matching terminal JSONL result."""
    if not command or not all(isinstance(item, str) and item for item in command):
        raise PluginProtocolError("plugin command must be a non-empty argv array")
    request_id = str(uuid4())
    request = {
        "api": PLUGIN_API,
        "id": request_id,
        "method": method,
        "params": dict(params),
    }
    try:
        encoded = (
            json.dumps(
                request,
                ensure_ascii=False,
                allow_nan=False,
                separators=(",", ":"),
            )
            + "\n"
        )
    except (TypeError, ValueError) as exc:
        raise PluginProtocolError(f"plugin request is not valid JSON: {exc}") from exc
    resolved_command = _resolved_command(command, environment)
    try:
        completed = executor(
            resolved_command,
            cwd=cwd,
            env=dict(environment),
            input=encoded,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="strict",
            check=False,
        )
    except (OSError, UnicodeError) as exc:
        raise ToolError(f"cannot invoke plugin {command[0]}: {exc}") from exc
    output = completed.stdout
    if not isinstance(output, str):
        raise PluginProtocolError("plugin stdout was not UTF-8 text")
    exit_code = int(completed.returncode)
    try:
        response = parse_plugin_output(request_id, output, exit_code=exit_code)
    except PluginProtocolError as exc:
        if method == "invoke":
            raise PluginProtocolError(
                f"plugin invoke outcome is unknown: {exc}"
            ) from exc
        raise
    if response.ok and exit_code != 0:
        message = f"plugin reported success but exited with status {exit_code}"
        if method == "invoke":
            message = f"plugin invoke outcome is unknown: {message}"
        raise PluginProtocolError(message)
    return response


def _resolved_command(
    command: Sequence[str], environment: Mapping[str, str]
) -> list[str]:
    """Resolve Windows batch shims while keeping direct argv execution."""
    values = list(command)
    if os.name != "nt":
        return values
    candidate = shutil.which(values[0], path=environment.get("PATH"))
    if candidate is not None and Path(candidate).suffix.casefold() in {".bat", ".cmd"}:
        values[0] = candidate
    return values


def parse_plugin_output(
    request_id: str, output: str, *, exit_code: int
) -> PluginResponse:
    """Validate JSONL events and a single terminal result for one request."""
    events: list[dict[str, JsonValue]] = []
    terminal: dict[str, JsonValue] | None = None
    for line_number, line in enumerate(split_jsonl(output), start=1):
        if not line.strip():
            raise PluginProtocolError(
                f"plugin emitted an empty JSONL frame on line {line_number}"
            )
        try:
            value: object = cast(
                object,
                json.loads(
                    line,
                    object_pairs_hook=_unique_object,
                    parse_constant=_reject_json_constant,
                    parse_float=_parse_finite_float,
                ),
            )
        except (json.JSONDecodeError, ValueError) as exc:
            raise PluginProtocolError(
                f"plugin emitted invalid JSON on line {line_number}: {exc}"
            ) from exc
        if not isinstance(value, dict):
            raise PluginProtocolError(
                f"plugin line {line_number} must be a JSON object"
            )
        typed = _json_object(cast(dict[str, object], value), line_number)
        message_id = typed.get("id")
        if message_id is not None and message_id != request_id:
            raise PluginProtocolError(
                f"plugin line {line_number} has the wrong request id"
            )
        frame_type = typed.get("type")
        if frame_type == "result":
            if terminal is not None:
                raise PluginProtocolError(
                    "plugin emitted more than one terminal result"
                )
            if message_id != request_id:
                raise PluginProtocolError(
                    "plugin terminal result is missing the request id"
                )
            terminal = typed
        elif frame_type == "event":
            if terminal is not None:
                raise PluginProtocolError(
                    "plugin emitted an event after its terminal result"
                )
            if message_id != request_id:
                raise PluginProtocolError("plugin event is missing the request id")
            event = typed.get("event")
            if not isinstance(event, dict):
                raise PluginProtocolError("plugin event must contain an `event` object")
            events.append(typed)
        else:
            raise PluginProtocolError(
                f"plugin line {line_number} has unknown type {frame_type!r}"
            )
    if terminal is None:
        detail = f" (process exit {exit_code})" if exit_code else ""
        raise PluginProtocolError(f"plugin emitted no terminal result{detail}")

    ok = terminal.get("ok")
    if not isinstance(ok, bool):
        raise PluginProtocolError("plugin terminal `ok` must be boolean")
    if ok:
        if "value" not in terminal or "error" in terminal:
            raise PluginProtocolError(
                "successful plugin terminal must contain value and no error"
            )
        result_value = terminal["value"]
        result_error = None
    else:
        if "error" not in terminal or "value" in terminal:
            raise PluginProtocolError(
                "failed plugin terminal must contain error and no value"
            )
        result_value = None
        result_error = terminal["error"]
    return PluginResponse(
        request_id=request_id,
        ok=ok,
        value=result_value,
        error=result_error,
        events=tuple(events),
        exit_code=exit_code,
    )


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


def _json_object(value: dict[str, object], line_number: int) -> dict[str, JsonValue]:
    converted: dict[str, JsonValue] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise PluginProtocolError(
                f"plugin line {line_number} contains a non-text key"
            )
        converted[key] = _json_value(item, line_number)
    return converted


def _json_value(value: object, line_number: int) -> JsonValue:
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, list):
        items = cast(list[object], value)
        return [_json_value(item, line_number) for item in items]
    if isinstance(value, dict):
        return _json_object(cast(dict[str, object], value), line_number)
    raise PluginProtocolError(f"plugin line {line_number} contains a non-JSON value")


__all__ = [
    "PluginExecutor",
    "PluginResponse",
    "invoke_plugin",
    "parse_plugin_output",
]
