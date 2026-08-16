from __future__ import annotations

import io
import json

import pytest

from tactus_runtime.plugin_protocol import (
    API_VERSION,
    EventWriter,
    PluginError,
    PluginRequest,
    parse_request,
    run_plugin,
)


def _run(
    request: object,
    handler: object,
) -> tuple[int, list[dict[str, object]], str]:
    stdin = io.StringIO(json.dumps(request, ensure_ascii=False))
    stdout = io.StringIO()
    stderr = io.StringIO()
    exit_code = run_plugin(handler, stdin=stdin, stdout=stdout, stderr=stderr)  # type: ignore[arg-type]
    lines = [json.loads(line) for line in stdout.getvalue().splitlines()]
    return exit_code, lines, stderr.getvalue()


def test_valid_request_emits_events_then_exactly_one_terminal() -> None:
    request = {
        "api": API_VERSION,
        "id": "请求-1",
        "method": "demo",
        "params": {"value": "你好"},
    }

    def handler(value: PluginRequest, writer: EventWriter) -> object:
        writer.event("progress", step=1)
        writer.event("future.event", raw={"unknown": True})
        writer.event(
            "result",
            id="provider-owned-id",
            note="nested subtype, not a terminal frame",
        )
        return {"echo": value.params["value"]}

    exit_code, lines, diagnostics = _run(request, handler)

    assert exit_code == 0
    assert diagnostics == ""
    assert [line["type"] for line in lines] == ["event", "event", "event", "result"]
    assert all(line["id"] == "请求-1" for line in lines)
    assert lines[0]["event"] == {"type": "progress", "step": 1}
    assert lines[1]["event"] == {
        "type": "future.event",
        "raw": {"unknown": True},
    }
    assert lines[2]["event"] == {
        "type": "result",
        "id": "provider-owned-id",
        "note": "nested subtype, not a terminal frame",
    }
    assert lines[-1] == {
        "type": "result",
        "id": "请求-1",
        "ok": True,
        "value": {"echo": "你好"},
    }


def test_expected_failure_is_a_single_failed_terminal() -> None:
    request = {
        "api": API_VERSION,
        "id": 7,
        "method": "demo",
        "params": {},
    }

    def handler(_request: PluginRequest, writer: EventWriter) -> object:
        writer.event("before.failure")
        raise PluginError("demo_failed", "expected failure", details={"retry": False})

    exit_code, lines, diagnostics = _run(request, handler)

    assert exit_code == 1
    assert [line["type"] for line in lines] == ["event", "result"]
    assert lines[0]["event"] == {"type": "before.failure"}
    assert lines[-1]["ok"] is False
    assert lines[-1]["error"] == {
        "code": "demo_failed",
        "message": "expected failure",
        "details": {"retry": False},
    }
    assert "demo_failed" in diagnostics


def test_invalid_json_still_emits_one_terminal_result() -> None:
    stdout = io.StringIO()
    stderr = io.StringIO()

    exit_code = run_plugin(
        lambda _request, _writer: None,
        stdin=io.StringIO("{not json"),
        stdout=stdout,
        stderr=stderr,
    )
    lines = [json.loads(line) for line in stdout.getvalue().splitlines()]

    assert exit_code == 2
    assert len(lines) == 1
    assert lines[0]["type"] == "result"
    assert lines[0]["id"] is None
    assert lines[0]["ok"] is False
    assert lines[0]["error"]["code"] == "invalid_json"
    assert "stdin" in stderr.getvalue()


@pytest.mark.parametrize(
    "encoded",
    [
        '{"api":"agenstro.plugin/v1","id":"a","id":"b","method":"m","params":{}}',
        '{"api":"agenstro.plugin/v1","id":"a","method":"m","params":{"n":NaN}}',
        '{"api":"agenstro.plugin/v1","id":"a","method":"m","params":{"n":Infinity}}',
        '{"api":"agenstro.plugin/v1","id":"a","method":"m","params":{"n":1e999}}',
        '{"api":"agenstro.plugin/v1","id":"a","method":"m","params":{"n":1e-999}}',
    ],
)
def test_request_rejects_duplicate_keys_and_non_finite_numbers(encoded: str) -> None:
    stdout = io.StringIO()
    stderr = io.StringIO()

    exit_code = run_plugin(
        lambda _request, _writer: None,
        stdin=io.StringIO(encoded),
        stdout=stdout,
        stderr=stderr,
    )
    [terminal] = [json.loads(line) for line in stdout.getvalue().splitlines()]

    assert exit_code == 2
    assert terminal["ok"] is False
    assert terminal["error"]["code"] == "invalid_json"


def test_non_json_handler_value_becomes_one_internal_error_terminal() -> None:
    request = {
        "api": API_VERSION,
        "id": "bad-value",
        "method": "demo",
        "params": {},
    }

    exit_code, lines, diagnostics = _run(
        request,
        lambda _request, _writer: {"not-json": {1, 2, 3}},
    )

    assert exit_code == 1
    assert len(lines) == 1
    assert lines[0]["type"] == "result"
    assert lines[0]["id"] == "bad-value"
    assert lines[0]["ok"] is False
    assert lines[0]["error"]["code"] == "internal_error"
    assert "TypeError" in diagnostics


def test_parse_request_rejects_wrong_api_and_boolean_id() -> None:
    for request in (
        {"api": "future/v2", "id": "x", "method": "m", "params": {}},
        {"api": API_VERSION, "id": True, "method": "m", "params": {}},
    ):
        try:
            parse_request(request)
        except PluginError as exc:
            assert exc.code in {"unsupported_api", "invalid_request"}
        else:  # pragma: no cover - assertion branch
            raise AssertionError("invalid request was accepted")
