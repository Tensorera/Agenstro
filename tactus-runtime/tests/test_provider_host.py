from __future__ import annotations

import io
import json
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

from tactus_runtime import provider_host
from tactus_runtime.plugin_protocol import API_VERSION


def _fake_cli(tmp_path: Path) -> tuple[Path, Path]:
    script = tmp_path / "fake_agent_cli.py"
    log = tmp_path / "fake-agent.jsonl"
    script.write_text(
        textwrap.dedent(
            """
            import json
            import os
            import sys
            import time
            from pathlib import Path

            log_path = Path(sys.argv[1])
            argv = sys.argv[2:]
            prompt = sys.stdin.buffer.read().decode("utf-8")
            record = {
                "argv": argv,
                "stdin": prompt,
                "cwd": os.getcwd(),
                "extra_env": os.environ.get("TACTUS_TEST_ENV"),
                "opencode_config": os.environ.get("OPENCODE_CONFIG_CONTENT"),
            }
            with log_path.open("a", encoding="utf-8") as stream:
                stream.write(json.dumps(record, ensure_ascii=False) + "\\n")

            if "--sleep" in argv:
                time.sleep(1)

            provider = argv[0]
            if "--version" in argv:
                print(f"{provider} fake 1.0")
                raise SystemExit(0)
            if argv[1:] in (["login", "status"], ["auth", "list"]):
                print("authenticated")
                raise SystemExit(0)
            if argv[1:] == ["auth", "status", "--json"]:
                print('{"loggedIn":true}')
                raise SystemExit(0)

            print(json.dumps({"type": "future.event", "new_field": 42}))
            print("fake agent diagnostic", file=sys.stderr)
            if "--fail" in argv:
                raise SystemExit(7)

            answer = "TACTUS_OK" if "TACTUS_OK" in prompt else f"{provider} answer"
            if provider == "codex":
                output_path = Path(argv[argv.index("-o") + 1])
                output_path.write_text(answer, encoding="utf-8")
                print(json.dumps({
                    "type": "item.completed",
                    "item": {"type": "agent_message", "text": "event fallback"},
                }))
            elif provider == "claude":
                print(json.dumps({
                    "type": "assistant",
                    "message": {"content": [{"type": "text", "text": "partial"}]},
                }))
                print(json.dumps({"type": "result", "result": answer}))
            else:
                parts = [answer] if answer == "TACTUS_OK" else ["open", "code answer"]
                for part in parts:
                    print(json.dumps({"type": "text", "part": {"text": part}}))
            """
        ).lstrip(),
        encoding="utf-8",
    )
    return script, log


def _request(method: str, params: dict[str, object], request_id: str = "req-1") -> str:
    return json.dumps(
        {
            "api": API_VERSION,
            "id": request_id,
            "method": method,
            "params": params,
        },
        ensure_ascii=False,
    )


def _run_host(
    provider: str,
    method: str,
    params: dict[str, object],
) -> tuple[int, list[dict[str, object]], str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    exit_code = provider_host.main(
        [provider],
        stdin=io.StringIO(_request(method, params)),
        stdout=stdout,
        stderr=stderr,
    )
    lines = [json.loads(line) for line in stdout.getvalue().splitlines()]
    return exit_code, lines, stderr.getvalue()


def _records(path: Path) -> list[dict[str, object]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]


def test_provider_host_uses_utf8_for_legacy_codepage_standard_streams(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    request_id = "请求-😀"
    raw_stdin = io.BytesIO(_request("describe", {}, request_id).encode("utf-8"))
    raw_stdout = io.BytesIO()
    stdin = io.TextIOWrapper(raw_stdin, encoding="cp936", errors="strict")
    stdout = io.TextIOWrapper(raw_stdout, encoding="cp936", errors="strict")

    with monkeypatch.context() as scoped:
        scoped.setattr(provider_host.sys, "stdin", stdin)
        scoped.setattr(provider_host.sys, "stdout", stdout)
        exit_code = provider_host.main(["codex"], stderr=io.StringIO())

    stdout.flush()
    [terminal] = [
        json.loads(line) for line in raw_stdout.getvalue().decode("utf-8").splitlines()
    ]

    assert exit_code == 0
    assert stdin.encoding == "utf-8"
    assert stdout.encoding == "utf-8"
    assert not stdin.closed
    assert not stdout.closed
    assert terminal["id"] == request_id
    assert terminal["ok"] is True


@pytest.mark.parametrize(
    ("provider", "executable", "expected_text", "full_bypass"),
    [
        ("codex", "codex", "codex answer", True),
        ("claude-code", "claude", "claude answer", True),
        ("opencode", "opencode", "opencode answer", False),
    ],
)
def test_provider_invoke_uses_exact_argv_stdin_and_environment(
    tmp_path: Path,
    provider: str,
    executable: str,
    expected_text: str,
    full_bypass: bool,
) -> None:
    fake_cli, log = _fake_cli(tmp_path)
    prompt = "处理 Unicode 路径, 并回答。"
    params: dict[str, object] = {
        "workspace": str(tmp_path),
        "prompt": prompt,
        "model": "vendor/future-model",
        "effort": "future-ultra",
        "extra_args": ["--future-option", "enabled"],
        "extra_env": {"TACTUS_TEST_ENV": "环境-ok"},
        "options": {
            "command_prefix": [sys.executable, str(fake_cli), str(log)],
            "timeout_seconds": 10,
            "unknown_future_option": {"kept_open": True},
        },
    }

    exit_code, lines, diagnostics = _run_host(provider, "invoke", params)

    assert exit_code == 0
    terminals = [line for line in lines if line["type"] == "result"]
    assert len(terminals) == 1
    terminal = terminals[0]
    assert terminal["ok"] is True
    assert terminal["value"]["text"] == expected_text
    assert terminal["value"]["full_bypass"] is full_bypass
    assert any(
        line["type"] == "event"
        and line["event"]["type"] == "provider.raw"
        and line["event"]["raw"]["type"] == "future.event"
        for line in lines
    )
    assert "fake agent diagnostic" in diagnostics

    [record] = _records(log)
    argv = record["argv"]
    assert argv[0] == executable
    assert record["stdin"] == prompt
    assert Path(record["cwd"]) == tmp_path.resolve()
    assert record["extra_env"] == "环境-ok"
    assert argv[-3:-1] == ["--future-option", "enabled"] or argv[-2:] == [
        "--future-option",
        "enabled",
    ]

    if provider == "codex":
        assert argv[1] == "exec"
        assert "--dangerously-bypass-approvals-and-sandbox" in argv
        assert "--json" in argv
        assert "--skip-git-repo-check" in argv
        assert "--ephemeral" in argv
        assert argv[argv.index("-c") + 1] == 'model_reasoning_effort="future-ultra"'
        assert "-o" in argv
        assert argv[-1] == "-"
    elif provider == "claude-code":
        assert argv[1] == "-p"
        assert "--dangerously-skip-permissions" in argv
        assert argv[argv.index("--output-format") + 1] == "stream-json"
        assert argv[argv.index("--effort") + 1] == "future-ultra"
        assert "--no-session-persistence" in argv
    else:
        assert argv[1] == "run"
        assert "--auto" in argv
        assert argv[argv.index("--format") + 1] == "json"
        assert argv[argv.index("--variant") + 1] == "future-ultra"
        assert json.loads(record["opencode_config"])["permission"] == "allow"
        assert terminal["value"]["full_bypass"] is False
        assert "cannot be guaranteed" in terminal["value"]["warning"]


def test_claude_alias_describes_canonical_provider() -> None:
    exit_code, lines, diagnostics = _run_host("claude", "describe", {})

    assert exit_code == 0
    assert diagnostics == ""
    assert lines == [
        {
            "type": "result",
            "id": "req-1",
            "ok": True,
            "value": {
                "api": API_VERSION,
                "kind": "provider",
                "name": "claude-code",
                "implementation_version": "0.3.0",
                "aliases": ["claude"],
                "executable": "claude",
                "methods": ["describe", "smoke", "invoke"],
                "operations": ["describe", "smoke", "invoke"],
                "full_bypass": True,
                "reasoning_parameter": "effort",
                "options_schema": {
                    "type": "object",
                    "additionalProperties": True,
                    "properties": {
                        "command_prefix": {
                            "type": "array",
                            "items": {"type": "string"},
                        },
                        "timeout_seconds": {
                            "type": "number",
                            "exclusiveMinimum": 0,
                        },
                        "extra_args": {
                            "type": "array",
                            "items": {"type": "string"},
                        },
                        "extra_env": {
                            "type": "object",
                            "additionalProperties": {"type": "string"},
                        },
                        "auth_status": {"type": "boolean"},
                    },
                },
            },
        }
    ]


def test_smoke_resolves_platform_command_shim_without_a_shell(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = str(tmp_path / "codex.cmd")
    observed: list[list[str]] = []

    monkeypatch.setattr(
        provider_host.shutil,
        "which",
        lambda executable, *, path: resolved if executable == "codex" else None,
    )

    def fake_run(
        argv: list[str], **_kwargs: object
    ) -> subprocess.CompletedProcess[str]:
        observed.append(argv)
        return subprocess.CompletedProcess(
            argv, 0, stdout="codex fake 1.0\n", stderr=""
        )

    monkeypatch.setattr(provider_host.subprocess, "run", fake_run)

    exit_code, lines, diagnostics = _run_host(
        "codex",
        "smoke",
        {"workspace": str(tmp_path)},
    )

    assert exit_code == 0
    assert diagnostics == ""
    assert observed == [[resolved, "--version"]]
    assert lines[-1]["value"]["version"] == "codex fake 1.0"


def test_opencode_failure_preserves_unknown_events_and_bypass_caveat(
    tmp_path: Path,
) -> None:
    fake_cli, log = _fake_cli(tmp_path)
    params = {
        "workspace": str(tmp_path),
        "prompt": "fail after one future event",
        "options": {
            "command_prefix": [sys.executable, str(fake_cli), str(log)],
            "extra_args": ["--fail"],
            "extra_env": {"TACTUS_TEST_ENV": "nested-options"},
        },
    }

    exit_code, lines, diagnostics = _run_host("opencode", "invoke", params)

    assert exit_code == 1
    assert lines[0]["type"] == "event"
    assert lines[0]["event"]["type"] == "provider.raw"
    terminal = lines[-1]
    assert terminal["type"] == "result"
    assert terminal["ok"] is False
    assert terminal["error"]["code"] == "outcome_unknown"
    assert terminal["error"]["details"]["cause"] == "provider_exit"
    assert terminal["error"]["details"]["exit_code"] == 7
    assert terminal["error"]["details"]["full_bypass"] is False
    assert "cannot be guaranteed" in terminal["error"]["details"]["warning"]
    assert "fake agent diagnostic" in diagnostics
    assert _records(log)[0]["extra_env"] == "nested-options"


def test_provider_timeout_is_reported_as_outcome_unknown(tmp_path: Path) -> None:
    fake_cli, log = _fake_cli(tmp_path)
    params = {
        "workspace": str(tmp_path),
        "prompt": "the request may already have escaped",
        "options": {
            "command_prefix": [sys.executable, str(fake_cli), str(log)],
            "extra_args": ["--sleep"],
            "timeout_seconds": 0.05,
        },
    }

    exit_code, lines, diagnostics = _run_host("codex", "invoke", params)

    assert exit_code == 1
    assert diagnostics
    terminal = lines[-1]
    assert terminal["ok"] is False
    assert terminal["error"]["code"] == "outcome_unknown"
    assert terminal["error"]["details"]["cause"] == "timeout"


def test_smoke_is_version_only_unless_live_is_explicit(tmp_path: Path) -> None:
    fake_cli, log = _fake_cli(tmp_path)
    common: dict[str, object] = {
        "workspace": str(tmp_path),
        "options": {"command_prefix": [sys.executable, str(fake_cli), str(log)]},
    }

    exit_code, lines, _diagnostics = _run_host("claude", "smoke", common)

    assert exit_code == 0
    assert lines[-1]["value"]["live"] is False
    assert lines[-1]["value"]["text"] == "claude fake 1.0"
    assert len(_records(log)) == 1

    live = dict(common)
    live["live"] = True
    live["effort"] = "xhigh-next"
    exit_code, lines, _diagnostics = _run_host("claude-code", "smoke", live)

    assert exit_code == 0
    assert lines[-1]["value"]["live"] is True
    assert lines[-1]["value"]["text"] == "TACTUS_OK"
    assert len(_records(log)) == 3
    live_argv = _records(log)[-1]["argv"]
    assert live_argv[live_argv.index("--effort") + 1] == "xhigh-next"
    assert live_argv[live_argv.index("--tools") + 1] == ""
