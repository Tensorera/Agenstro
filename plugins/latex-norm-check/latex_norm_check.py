#!/usr/bin/env python3
"""Reference agenstro.plugin/v1 checker for text-shaped LaTeX norms."""

from __future__ import annotations

import json
import math
import os
import re
import subprocess
import sys
from collections.abc import Iterator
from decimal import Decimal, InvalidOperation
from typing import Any

PLUGIN_API = "agenstro.plugin/v1"
NORM_API = "agenstro.norm/v1"
IMPLEMENTATION = "latex-norm-check"
VERSION = "0.1.0"

SUPPORTED_SPECS = ("Existence", "Absence", "Occurrence", "Consistency", "Metric")
SUPPORTED_METRICS = ("characters", "lines", "display-equations")
SEVERITIES = ("Preference", "Style", "Correctness", "Blocking")
MAX_VIOLATIONS_PER_NORM = 50
MAX_SNIPPET_CHARS = 160
MAX_NORM_SOURCE_BYTES = 512 * 1024
MAX_PATTERN_BYTES = 4 * 1024
MAX_PLUGIN_REQUEST_BYTES = 1024 * 1024
REGEX_DEADLINE_SECONDS = 1.0
MIN_JSON_INTEGER = -(2**63)
MAX_JSON_INTEGER = 2**64 - 1
MIN_CORRELATION_INTEGER = -(2**63)
MAX_CORRELATION_INTEGER = 2**63 - 1


class UnsupportedCheck(Exception):
    """The spec shape is valid, but this checker cannot evaluate it."""


class JsonDomainError(ValueError):
    """Input is outside Agenstro's strict JSON domain."""


def emit(frame: dict[str, Any]) -> None:
    sys.stdout.write(
        json.dumps(frame, ensure_ascii=False, allow_nan=False, separators=(",", ":")) + "\n"
    )
    sys.stdout.flush()


def event(request_id: str | int, payload: dict[str, Any]) -> None:
    emit({"type": "event", "id": request_id, "event": payload})


def ok(request_id: str | int, value: Any) -> None:
    emit({"type": "result", "id": request_id, "ok": True, "value": value})


def failed(request_id: str | int, code: str, message: str, details: Any = None) -> None:
    error: dict[str, Any] = {"code": code, "message": message}
    if details is not None:
        error["details"] = details
    emit({"type": "result", "id": request_id, "ok": False, "error": error})


def line_column(text: str, offset: int) -> tuple[int, int]:
    """Return a one-based line and column for a Python string offset."""
    prefix = text[:offset]
    line = prefix.count("\n") + 1
    column = offset - (prefix.rfind("\n") + 1) + 1
    return line, column


def match_locus(artifact: str, text: str, start: int, end: int) -> dict[str, Any]:
    start_line, start_column = line_column(text, start)
    # Python match.end() is exclusive while agenstro.norm/v1 coordinates are
    # inclusive.  Point an empty match at its start; otherwise use the final
    # character that belongs to the match.
    inclusive_end = start if end == start else end - 1
    end_line, end_column = line_column(text, inclusive_end)
    return {
        "artifact": artifact,
        "startLine": start_line,
        "startColumn": start_column,
        "endLine": end_line,
        "endColumn": end_column,
        "snippet": text[start:end][:MAX_SNIPPET_CHARS],
    }


def compile_pattern(pattern: str, ignore_case: bool) -> re.Pattern[str]:
    return re.compile(pattern, re.MULTILINE | (re.IGNORECASE if ignore_case else 0))


def regex_worker() -> int:
    """Evaluate one already shape-checked spec in a killable subprocess."""
    try:
        payload = json.loads(sys.stdin.buffer.read().decode("utf-8"))
        operation = payload["operation"]
        text = payload["text"]
        if operation == "spans":
            regex = compile_pattern(payload["pattern"], payload["ignoreCase"])
            spans: list[list[int]] = []
            truncated = False
            for index, match in enumerate(regex.finditer(text)):
                if index >= MAX_VIOLATIONS_PER_NORM:
                    truncated = True
                    break
                spans.append([match.start(), match.end()])
            value: Any = {"spans": spans, "truncated": truncated}
        elif operation == "search":
            regex = compile_pattern(payload["pattern"], payload["ignoreCase"])
            value = {"matched": regex.search(text) is not None}
        elif operation == "count":
            regex = compile_pattern(payload["pattern"], False)
            value = {"count": sum(1 for _ in regex.finditer(text))}
        elif operation == "consistency":
            value = {
                "present": [
                    [variant for variant in group if compile_pattern(variant, True).search(text)]
                    for group in payload["groups"]
                ]
            }
        else:
            raise ValueError(f"unknown regex worker operation: {operation}")
        sys.stdout.write(json.dumps({"ok": True, "value": value}, ensure_ascii=True))
    except (KeyError, TypeError, ValueError, re.error) as error:
        sys.stdout.write(json.dumps({"ok": False, "error": str(error)}, ensure_ascii=True))
    return 0


def evaluate_regex(payload: dict[str, Any]) -> dict[str, Any]:
    encoded = json.dumps(payload, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
    try:
        completed = subprocess.run(
            [sys.executable, os.path.abspath(__file__), "--regex-worker"],
            input=encoded,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=REGEX_DEADLINE_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise UnsupportedCheck(
            f"regular expression evaluation exceeded {REGEX_DEADLINE_SECONDS:g} second"
        ) from error
    if completed.returncode != 0:
        raise UnsupportedCheck(f"regular expression worker exited with {completed.returncode}")
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise UnsupportedCheck("regular expression worker returned invalid JSON") from error
    if not isinstance(result, dict) or not result.get("ok"):
        message = (
            result.get("error", "regular expression worker failed")
            if isinstance(result, dict)
            else "regular expression worker failed"
        )
        raise UnsupportedCheck(str(message))
    value = result.get("value")
    if not isinstance(value, dict):
        raise UnsupportedCheck("regular expression worker returned an invalid value")
    return value


def strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise JsonDomainError(f"duplicate object key: {key}")
        result[key] = value
    return result


def strict_integer(raw: str) -> int:
    value = int(raw)
    if value < MIN_JSON_INTEGER or value > MAX_JSON_INTEGER:
        raise JsonDomainError("integer is outside the signed-64/unsigned-64 JSON domain")
    return value


def strict_float(raw: str) -> float:
    try:
        exact = Decimal(raw)
        value = float(exact)
    except (InvalidOperation, OverflowError) as error:
        raise JsonDomainError("invalid floating-point number") from error
    if not exact.is_finite() or not math.isfinite(value):
        raise JsonDomainError("floating-point number is non-finite or overflowed")
    if exact != 0 and value == 0:
        raise JsonDomainError("floating-point number underflowed to zero")
    return value


def reject_constant(raw: str) -> Any:
    raise JsonDomainError(f"non-standard numeric constant: {raw}")


def decode_strict_json(raw: str) -> Any:
    return json.loads(
        raw,
        object_pairs_hook=strict_object,
        parse_int=strict_integer,
        parse_float=strict_float,
        parse_constant=reject_constant,
    )


def is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def utf8_length(value: str) -> int:
    return len(value.encode("utf-8"))


def validate_pattern(pattern: Any) -> str:
    if not isinstance(pattern, str):
        raise TypeError("specPattern must be a string")
    if not pattern:
        raise TypeError("specPattern must not be empty")
    if utf8_length(pattern) > MAX_PATTERN_BYTES:
        raise TypeError(f"specPattern exceeds the {MAX_PATTERN_BYTES}-byte limit")
    return pattern


def validate_bound(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise TypeError("specBound must be an object")
    low = value.get("boundMinimum")
    high = value.get("boundMaximum")
    if low is None and high is None:
        raise TypeError("specBound must define at least one non-null endpoint")
    if low is not None and not is_number(low):
        raise TypeError("boundMinimum must be a finite number or null")
    if high is not None and not is_number(high):
        raise TypeError("boundMaximum must be a finite number or null")
    if low is not None and high is not None and low > high:
        raise TypeError("boundMinimum must not exceed boundMaximum")
    return value


def validate_check_spec(spec: dict[str, Any], kind: str) -> None:
    if kind in ("Existence", "Absence", "Occurrence"):
        validate_pattern(spec.get("specPattern"))
    if kind in ("Existence", "Absence"):
        ignore_case = spec.get("specIgnoreCase", False)
        if not isinstance(ignore_case, bool):
            raise TypeError("specIgnoreCase must be a boolean")
    elif kind == "Occurrence":
        if "specBound" not in spec:
            raise TypeError("Occurrence requires specBound")
        validate_bound(spec["specBound"])
    elif kind == "Consistency":
        groups = spec.get("specGroups")
        if not isinstance(groups, list) or not groups:
            raise TypeError("specGroups must be an array of string arrays")
        for index, group in enumerate(groups):
            if not isinstance(group, list) or len(group) < 2:
                raise TypeError(f"specGroups[{index}] must contain at least two distinct patterns")
            patterns = [validate_pattern(item) for item in group]
            if len(set(patterns)) < 2:
                raise TypeError(f"specGroups[{index}] must contain at least two distinct patterns")
    elif kind == "Metric":
        if not isinstance(spec.get("specMetric"), str):
            raise TypeError("specMetric must be a string")
        if "specBound" not in spec:
            raise TypeError("Metric requires specBound")
        validate_bound(spec["specBound"])


def check_absence(
    norm: dict[str, Any], spec: dict[str, Any], text: str, artifact: str
) -> Iterator[dict[str, Any]]:
    evaluated = evaluate_regex(
        {
            "operation": "spans",
            "pattern": spec["specPattern"],
            "ignoreCase": spec.get("specIgnoreCase", False),
            "text": text,
        }
    )
    spans = evaluated.get("spans")
    if not isinstance(spans, list):
        raise UnsupportedCheck("regular expression worker omitted match spans")
    for span in spans:
        if (
            not isinstance(span, list)
            or len(span) != 2
            or not all(isinstance(offset, int) for offset in span)
        ):
            raise UnsupportedCheck("regular expression worker returned an invalid match span")
        start, end = span
        yield {
            "norm": norm["id"],
            "severity": norm["severity"],
            "message": norm["statement"],
            "locus": match_locus(artifact, text, start, end),
        }
    if evaluated.get("truncated") is True:
        event(
            CURRENT_REQUEST_ID,
            {
                "type": "norm.truncated",
                "norm": norm["id"],
                "limit": MAX_VIOLATIONS_PER_NORM,
            },
        )


def check_existence(
    norm: dict[str, Any], spec: dict[str, Any], text: str, artifact: str
) -> Iterator[dict[str, Any]]:
    evaluated = evaluate_regex(
        {
            "operation": "search",
            "pattern": spec["specPattern"],
            "ignoreCase": spec.get("specIgnoreCase", False),
            "text": text,
        }
    )
    matched = evaluated.get("matched")
    if not isinstance(matched, bool):
        raise UnsupportedCheck("regular expression worker omitted the search result")
    if not matched:
        yield {
            "norm": norm["id"],
            "severity": norm["severity"],
            "message": norm["statement"],
            "locus": {"artifact": artifact},
        }


def check_occurrence(
    norm: dict[str, Any], spec: dict[str, Any], text: str, artifact: str
) -> Iterator[dict[str, Any]]:
    evaluated = evaluate_regex(
        {"operation": "count", "pattern": spec["specPattern"], "text": text}
    )
    count = evaluated.get("count")
    if not isinstance(count, int):
        raise UnsupportedCheck("regular expression worker omitted the match count")
    bound = spec["specBound"]
    low, high = bound.get("boundMinimum"), bound.get("boundMaximum")
    if (low is not None and count < low) or (high is not None and count > high):
        yield {
            "norm": norm["id"],
            "severity": norm["severity"],
            "message": f"{norm['statement']} (found {count})",
            "locus": {"artifact": artifact},
            "evidence": {"count": count, "minimum": low, "maximum": high},
        }


def check_consistency(
    norm: dict[str, Any], spec: dict[str, Any], text: str, artifact: str
) -> Iterator[dict[str, Any]]:
    evaluated = evaluate_regex(
        {"operation": "consistency", "groups": spec["specGroups"], "text": text}
    )
    present_groups = evaluated.get("present")
    if not isinstance(present_groups, list):
        raise UnsupportedCheck("regular expression worker omitted consistency results")
    for present in present_groups:
        if not isinstance(present, list) or any(not isinstance(item, str) for item in present):
            raise UnsupportedCheck("regular expression worker returned invalid consistency results")
        if len(present) > 1:
            yield {
                "norm": norm["id"],
                "severity": norm["severity"],
                "message": f"{norm['statement']} (mixed: {', '.join(present)})",
                "locus": {"artifact": artifact},
                "evidence": {"present": present},
            }


def check_metric(
    norm: dict[str, Any], spec: dict[str, Any], text: str, artifact: str
) -> Iterator[dict[str, Any]]:
    metrics = {
        "characters": float(len(text)),
        "lines": float(text.count("\n") + 1),
        "display-equations": float(len(re.findall(r"\\begin\{(?:equation|align|gather)\*?\}", text))),
    }
    name = spec.get("specMetric", "")
    if name not in metrics:
        # Returning an empty iterator here would falsely classify the norm as
        # checked and passing.  Unsupported measurements are explicitly
        # unchecked instead.
        raise UnsupportedCheck(f"unsupported metric: {name}")
    measured = metrics[name]
    bound = spec["specBound"]
    low, high = bound.get("boundMinimum"), bound.get("boundMaximum")
    if (low is not None and measured < low) or (high is not None and measured > high):
        yield {
            "norm": norm["id"],
            "severity": norm["severity"],
            "message": f"{norm['statement']} (measured {measured:g})",
            "locus": {"artifact": artifact},
            "evidence": {
                "metric": name,
                "measured": measured,
                "minimum": low,
                "maximum": high,
            },
        }


CHECKERS = {
    "Absence": check_absence,
    "Existence": check_existence,
    "Occurrence": check_occurrence,
    "Consistency": check_consistency,
    "Metric": check_metric,
}

# The truncation event is emitted from the iterator so it precedes the
# norm.checked event.  A process handles exactly one request, so one explicit
# per-process correlation id is sufficient and avoids plumbing it through each
# checker signature.
CURRENT_REQUEST_ID: str | int = "unknown"


def method_describe(request_id: str | int) -> None:
    ok(
        request_id,
        {
            "implementation": IMPLEMENTATION,
            "version": VERSION,
            "kind": "plugin",
            "methods": ["describe", "check", "smoke"],
            "capabilities": {
                "specs": list(SUPPORTED_SPECS),
                "metrics": list(SUPPORTED_METRICS),
                "artifact": "latex",
            },
        },
    )


def method_smoke(request_id: str | int, params: dict[str, Any]) -> None:
    live = bool(params.get("live", False))
    detail = "no external dependency" if live else f"{IMPLEMENTATION} {VERSION}"
    ok(request_id, {"live": live, "ok": True, "detail": detail})


def validate_norms(norms: Any) -> tuple[list[dict[str, Any]] | None, str | None]:
    if not isinstance(norms, list):
        return None, "check requires a 'norms' array"
    identities: set[str] = set()
    for index, norm in enumerate(norms):
        if not isinstance(norm, dict):
            return None, f"norms[{index}] must be an object"
        identity = norm.get("id")
        if not isinstance(identity, str) or not identity:
            return None, f"norms[{index}].id must be a non-empty string"
        if identity in identities:
            return None, f"duplicate norm id: {identity}"
        identities.add(identity)
        if not isinstance(norm.get("statement"), str):
            return None, f"norms[{index}].statement must be a string"
        if norm.get("severity") not in SEVERITIES:
            return None, f"norms[{index}].severity is not recognised"
        spec = norm.get("spec")
        if spec is not None and not isinstance(spec, dict):
            return None, f"norms[{index}].spec must be an object when present"
    return norms, None


def method_check(request_id: str | int, params: dict[str, Any]) -> None:
    text = params.get("source")
    if not isinstance(text, str):
        failed(request_id, "invalid_params", "check requires a string 'source' field")
        return
    if utf8_length(text) > MAX_NORM_SOURCE_BYTES:
        failed(
            request_id,
            "invalid_params",
            f"check source exceeds the {MAX_NORM_SOURCE_BYTES}-byte limit",
        )
        return
    artifact = params.get("artifact", "artifact.tex")
    if not isinstance(artifact, str) or not artifact:
        failed(
            request_id,
            "invalid_params",
            "check requires a non-empty string 'artifact' field",
        )
        return
    norms, validation_error = validate_norms(params.get("norms"))
    if validation_error is not None or norms is None:
        failed(request_id, "invalid_params", validation_error or "invalid norms")
        return

    violations: list[dict[str, Any]] = []
    checked: list[str] = []
    unchecked: list[str] = []

    for norm in norms:
        identity = norm["id"]
        spec = norm.get("spec") or {}
        kind = spec.get("kind")
        checker = CHECKERS.get(kind) if isinstance(kind, str) else None
        if checker is None:
            unchecked.append(identity)
            continue
        try:
            validate_check_spec(spec, kind)
            found = list(checker(norm, spec, text, artifact))
        except (KeyError, TypeError, re.error, UnsupportedCheck) as error:
            unchecked.append(identity)
            event(
                request_id,
                {
                    "type": "norm.check_failed",
                    "norm": identity,
                    "message": str(error),
                },
            )
            continue
        checked.append(identity)
        violations.extend(found)
        event(request_id, {"type": "norm.checked", "norm": identity, "violations": len(found)})

    ok(
        request_id,
        {
            "api": NORM_API,
            "artifact": artifact,
            "violations": violations,
            "checked": checked,
            "unchecked": unchecked,
        },
    )


def main() -> int:
    global CURRENT_REQUEST_ID

    if sys.argv[1:] == ["--regex-worker"]:
        return regex_worker()

    raw_bytes = sys.stdin.buffer.readline(MAX_PLUGIN_REQUEST_BYTES + 2)
    request_bytes = raw_bytes[:-1] if raw_bytes.endswith(b"\n") else raw_bytes
    if len(request_bytes) > MAX_PLUGIN_REQUEST_BYTES:
        failed(
            "unknown",
            "invalid_request",
            f"request exceeds the {MAX_PLUGIN_REQUEST_BYTES}-byte limit",
        )
        return 1
    try:
        raw = request_bytes.decode("utf-8")
    except UnicodeDecodeError as error:
        failed("unknown", "invalid_request", f"request is not valid UTF-8: {error}")
        return 1
    if not raw.strip():
        failed("unknown", "invalid_request", "empty request")
        return 1
    try:
        request = decode_strict_json(raw)
    except (json.JSONDecodeError, JsonDomainError) as error:
        failed("unknown", "invalid_request", f"invalid JSON: {error}")
        return 1
    if not isinstance(request, dict):
        failed("unknown", "invalid_request", "request must be an object")
        return 1

    request_id_value = request.get("id", "unknown")
    request_id = (
        request_id_value
        if isinstance(request_id_value, str)
        or (
            isinstance(request_id_value, int)
            and not isinstance(request_id_value, bool)
            and MIN_CORRELATION_INTEGER <= request_id_value <= MAX_CORRELATION_INTEGER
        )
        else "unknown"
    )
    CURRENT_REQUEST_ID = request_id
    if set(request) != {"api", "id", "method", "params"}:
        failed(request_id, "invalid_request", "request must contain exactly api, id, method, params")
        return 1
    if request_id == "unknown" and request_id_value != "unknown":
        failed(request_id, "invalid_request", "id must be a string or signed 64-bit integer")
        return 1
    if request.get("api") != PLUGIN_API:
        failed(request_id, "invalid_request", f"api must be {PLUGIN_API}")
        return 1
    method = request.get("method")
    if not isinstance(method, str) or not method:
        failed(request_id, "invalid_request", "method must be a non-empty string")
        return 1
    params = request.get("params")
    if not isinstance(params, dict):
        failed(request_id, "invalid_params", "params must be an object")
        return 1

    if method == "describe":
        method_describe(request_id)
    elif method == "smoke":
        method_smoke(request_id, params)
    elif method == "check":
        method_check(request_id, params)
    else:
        failed(request_id, "unknown_method", f"unknown method '{method}'")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
