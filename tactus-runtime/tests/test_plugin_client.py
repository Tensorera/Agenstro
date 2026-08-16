from __future__ import annotations

import json
import os
import subprocess
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from tactus_runtime import cli
from tactus_runtime.errors import PluginProtocolError
from tactus_runtime.plugin_client import (
    _resolved_command,
    invoke_plugin,
    parse_plugin_output,
)
from tactus_runtime.workspace import initialize_workspace


@pytest.mark.parametrize(
    ("output", "message"),
    [
        ("", "no terminal result"),
        (
            '{"type":"result","id":"wrong","ok":true,"value":{}}\n',
            "wrong request id",
        ),
        (
            '{"type":"result","id":"request","ok":true,"value":{}}\n'
            '{"type":"result","id":"request","ok":true,"value":{}}\n',
            "more than one terminal",
        ),
        (
            '{"type":"result","id":"request","ok":true,"error":{}}\n',
            "must contain value",
        ),
        (
            '{"type":"result","id":"request","ok":false,"value":{}}\n',
            "must contain error",
        ),
        (
            '{"type":"result","id":"request","ok":true,"value":NaN}\n',
            "non-finite JSON number",
        ),
        (
            '{"type":"result","id":"request","ok":true,"value":1e999}\n',
            "outside the finite float range",
        ),
        (
            '{"type":"result","id":"request","ok":true,"value":1e-999}\n',
            "outside the finite float range",
        ),
        (
            '\n{"type":"result","id":"request","ok":true,"value":null}\n',
            "empty JSONL frame",
        ),
    ],
)
def test_plugin_terminal_validation(output: str, message: str) -> None:
    with pytest.raises(PluginProtocolError, match=message):
        parse_plugin_output("request", output, exit_code=0)


def test_plugin_response_preserves_preterminal_events() -> None:
    response = parse_plugin_output(
        "request",
        '{"type":"event","id":"request","event":{"type":"progress","step":1}}\n'
        '{"type":"result","id":"request","ok":true,"value":{"ready":true}}\n',
        exit_code=0,
    )

    assert response.ok is True
    assert response.value == {"ready": True}
    assert response.error is None
    assert response.events == (
        {
            "type": "event",
            "id": "request",
            "event": {"type": "progress", "step": 1},
        },
    )


@pytest.mark.parametrize("separator", ["\u0085", "\u2028", "\u2029"])
def test_plugin_jsonl_keeps_unicode_line_characters_inside_strings(
    separator: str,
) -> None:
    text = f"before{separator}after"
    event = json.dumps(
        {
            "type": "event",
            "id": "request",
            "event": {"type": "provider.raw", "text": text},
        },
        ensure_ascii=False,
    )
    terminal = json.dumps(
        {"type": "result", "id": "request", "ok": True, "value": {"text": text}},
        ensure_ascii=False,
    )

    response = parse_plugin_output(
        "request",
        event + "\n" + terminal + "\n",
        exit_code=0,
    )

    assert response.events[0]["event"]["text"] == text
    assert response.value == {"text": text}


@pytest.mark.parametrize(
    "output",
    [
        '{"type":"progress","id":"request","step":1}\n',
        '{"type":"event","id":"request"}\n',
        '{"type":"event","event":{"type":"progress"}}\n',
    ],
)
def test_plugin_response_rejects_noncanonical_event_frames(output: str) -> None:
    with pytest.raises(PluginProtocolError):
        parse_plugin_output(
            "request",
            output + '{"type":"result","id":"request","ok":true,"value":null}\n',
            exit_code=0,
        )


def test_invoke_success_with_nonzero_exit_is_outcome_unknown(tmp_path: Path) -> None:
    def inconsistent_plugin(
        command: Sequence[str],
        **kwargs: Any,
    ) -> subprocess.CompletedProcess[str]:
        request = json.loads(kwargs["input"])
        terminal = {
            "type": "result",
            "id": request["id"],
            "ok": True,
            "value": {"text": "possibly completed"},
        }
        return subprocess.CompletedProcess(
            command,
            9,
            stdout=json.dumps(terminal) + "\n",
        )

    with pytest.raises(PluginProtocolError, match="outcome is unknown"):
        invoke_plugin(
            ["fake-plugin"],
            method="invoke",
            params={},
            cwd=tmp_path,
            environment={},
            executor=inconsistent_plugin,
        )


@pytest.mark.parametrize("non_finite", [float("nan"), float("inf"), float("-inf")])
def test_invoke_rejects_non_finite_request_before_starting_plugin(
    tmp_path: Path,
    non_finite: float,
) -> None:
    def must_not_run(
        _command: Sequence[str],
        **_kwargs: Any,
    ) -> subprocess.CompletedProcess[str]:
        raise AssertionError("invalid JSON must not start the plugin")

    with pytest.raises(PluginProtocolError, match="request is not valid JSON"):
        invoke_plugin(
            ["fake-plugin"],
            method="invoke",
            params={"value": non_finite},
            cwd=tmp_path,
            environment={},
            executor=must_not_run,
        )


@pytest.mark.skipif(os.name != "nt", reason="Windows command shim behavior")
def test_plugin_command_resolves_windows_batch_shim(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        "tactus_runtime.plugin_client.shutil.which",
        lambda executable, *, path: (
            "C:/npm/plugin.cmd" if executable == "plugin" else None
        ),
    )

    assert _resolved_command(["plugin", "arg"], {"PATH": "C:/npm"}) == [
        "C:/npm/plugin.cmd",
        "arg",
    ]


class SuccessfulPlugin:
    def __init__(self) -> None:
        self.calls: list[tuple[list[str], dict[str, Any], dict[str, object]]] = []

    def __call__(
        self,
        command: Sequence[str],
        **kwargs: Any,
    ) -> subprocess.CompletedProcess[str]:
        request = json.loads(kwargs["input"])
        self.calls.append((list(command), dict(kwargs), request))
        terminal = {
            "type": "result",
            "id": request["id"],
            "ok": True,
            "value": {"ready": True},
        }
        return subprocess.CompletedProcess(
            command,
            0,
            stdout=json.dumps(terminal) + "\n",
        )


def test_smoke_uses_generic_jsonl_protocol_and_runtime_environment(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sdk = tmp_path / "clef-sdk"
    sdk.mkdir()
    root = tmp_path / "project"
    report = initialize_workspace(root, sdk=sdk)
    config = report.workspace.config_path.read_text(encoding="utf-8")
    report.workspace.config_path.write_text(
        config.replace(
            "[providers.codex]\ncommand = ",
            '[providers.codex]\nmodel = "future-model"\n'
            'effort = "future-effort"\ncommand = ',
        ).replace(
            '[providers."claude-code"]',
            "[providers.codex.options]\ntimeout_seconds = 7\n"
            'extra_env = { TACTUS_SMOKE = "configured" }\n\n'
            '[providers."claude-code"]',
        ),
        encoding="utf-8",
    )
    plugin = SuccessfulPlugin()

    assert (
        cli.main(
            ["smoke", "--root", str(root), "provider:codex", "--live", "--json"],
            plugin_executor=plugin,
        )
        == 0
    )
    output = json.loads(capsys.readouterr().out)

    assert output["api"] == "agenstro.plugin/v1"
    assert output["live"] is True
    assert output["plugins"][0]["ok"] is True
    assert len(plugin.calls) == 1
    command, options, request = plugin.calls[0]
    assert command == ["tactus-provider-host", "codex"]
    assert request == {
        "api": "agenstro.plugin/v1",
        "id": request["id"],
        "method": "smoke",
        "params": {
            "workspace": str(root.resolve()),
            "live": True,
            "model": "future-model",
            "effort": "future-effort",
            "options": {
                "timeout_seconds": 7,
                "extra_env": {"TACTUS_SMOKE": "configured"},
            },
        },
    }
    assert Path(options["env"]["TACTUS_RUNTIME_CONFIG"]).is_absolute()
    assert options["cwd"] == root.resolve()


def test_prompt_and_generate_inject_instructions_without_running_scripts(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sdk = tmp_path / "clef-sdk"
    sdk.mkdir()
    root = tmp_path / "project"
    report = initialize_workspace(root, sdk=sdk)
    report.workspace.prompt_path.write_text(
        "Keep the workflow statically checkable.\n",
        encoding="utf-8",
    )

    assert cli.main(["prompt", "--root", str(root)]) == 0
    assert capsys.readouterr().out == "Keep the workflow statically checkable.\n"

    plugin = SuccessfulPlugin()

    def generate_plugin(
        command: Sequence[str],
        **kwargs: Any,
    ) -> subprocess.CompletedProcess[str]:
        request = json.loads(kwargs["input"])
        plugin.calls.append((list(command), dict(kwargs), request))
        if request["method"] == "invoke":
            scripts = Path(kwargs["cwd"]) / ".tactus" / "scripts"
            (scripts / "010_plan.hs").write_text(
                "main = pure ()\n",
                encoding="utf-8",
            )
            (scripts / "020_execute.hs").write_text(
                "main = pure ()\n",
                encoding="utf-8",
            )
            value = {"text": "created two scripts"}
        elif request["method"] == "observe.begin":
            value = {"token": "opaque"}
        else:
            value = {"delta": {"added": [".tactus/scripts/010_plan.hs"]}}
        terminal = {
            "type": "result",
            "id": request["id"],
            "ok": True,
            "value": value,
        }
        return subprocess.CompletedProcess(
            command,
            0,
            stdout=json.dumps(terminal) + "\n",
        )

    assert (
        cli.main(
            [
                "generate",
                "--root",
                str(root),
                "build",
                "a",
                "workflow",
                "--provider",
                "codex",
                "--json",
            ],
            plugin_executor=generate_plugin,
        )
        == 0
    )
    output = json.loads(capsys.readouterr().out)
    assert [call[2]["method"] for call in plugin.calls] == [
        "observe.begin",
        "invoke",
        "observe.end",
    ]
    begin_request = plugin.calls[0][2]
    request = plugin.calls[1][2]
    end_request = plugin.calls[2][2]

    assert request["method"] == "invoke"
    assert request["params"]["workspace"] == str(root.resolve())
    assert request["params"]["model"] is None
    assert request["params"]["effort"] is None
    assert request["params"]["options"] == {}
    assert "Keep the workflow statically checkable." in request["params"]["prompt"]
    assert "Goal: build a workflow" in request["params"]["prompt"]
    assert ".tactus/scripts/" in request["params"]["prompt"]
    assert begin_request["params"]["invocation"] == end_request["params"]["invocation"]
    assert end_request["params"]["begin"] == {"token": "opaque"}
    assert end_request["params"]["outcome"] == "ok"
    assert [entry["method"] for entry in output["effects"]] == [
        "observe.begin",
        "observe.end",
    ]
    assert output["observer_errors"] == []
    assert [script["path"] for script in output["scripts"]] == [
        ".tactus/scripts/010_plan.hs",
        ".tactus/scripts/020_execute.hs",
    ]


def test_generate_preserves_effect_evidence_when_provider_outcome_is_unknown(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sdk = tmp_path / "clef-sdk"
    sdk.mkdir()
    root = tmp_path / "project"
    initialize_workspace(root, sdk=sdk)
    calls: list[str] = []

    def broken_after_write(
        command: Sequence[str],
        **kwargs: Any,
    ) -> subprocess.CompletedProcess[str]:
        request = json.loads(kwargs["input"])
        method = request["method"]
        calls.append(method)
        if method == "invoke":
            script = Path(kwargs["cwd"]) / ".tactus" / "scripts" / "010_partial.hs"
            script.write_text("main = pure ()\n", encoding="utf-8")
            event = {
                "type": "event",
                "id": request["id"],
                "event": {"type": "provider.raw", "text": "wrote then crashed"},
            }
            return subprocess.CompletedProcess(
                command,
                0,
                stdout=json.dumps(event) + "\n",
            )
        value = (
            {"token": "opaque"}
            if method == "observe.begin"
            else {"delta": {"added": [".tactus/scripts/010_partial.hs"]}}
        )
        terminal = {
            "type": "result",
            "id": request["id"],
            "ok": True,
            "value": value,
        }
        return subprocess.CompletedProcess(
            command,
            0,
            stdout=json.dumps(terminal) + "\n",
        )

    assert (
        cli.main(
            ["generate", "--root", str(root), "partial", "write", "--json"],
            plugin_executor=broken_after_write,
        )
        == 1
    )
    output = json.loads(capsys.readouterr().out)

    assert calls == ["observe.begin", "invoke", "observe.end"]
    assert output["ok"] is False
    assert output["error"]["code"] == "outcome_unknown"
    assert "outcome is unknown" in output["error"]["message"]
    assert [item["method"] for item in output["effects"]] == [
        "observe.begin",
        "observe.end",
    ]
    assert output["effects"][-1]["value"]["delta"]["added"] == [
        ".tactus/scripts/010_partial.hs"
    ]
    assert output["scripts"][0]["path"] == ".tactus/scripts/010_partial.hs"


def test_smoke_rejects_a_plugin_without_a_terminal(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sdk = tmp_path / "clef-sdk"
    sdk.mkdir()
    root = tmp_path / "project"
    initialize_workspace(root, sdk=sdk)

    def no_terminal(
        command: Sequence[str],
        **_kwargs: Any,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(command, 0, stdout="")

    assert (
        cli.main(
            ["smoke", "--root", str(root), "codex", "--json"],
            plugin_executor=no_terminal,
        )
        == 1
    )
    output = json.loads(capsys.readouterr().out)
    assert output["plugins"][0]["ok"] is False
    assert "no terminal result" in output["plugins"][0]["error"]
