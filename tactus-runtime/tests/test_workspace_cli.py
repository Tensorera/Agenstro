from __future__ import annotations

import json
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

from tactus_runtime import cli
from tactus_runtime.runner import effective_path
from tactus_runtime.workspace import (
    discover_scripts,
    explicit_scripts,
    initialize_workspace,
    open_workspace,
)


class RecordingExecutor:
    def __init__(self, statuses: Sequence[int] = ()) -> None:
        self.statuses = list(statuses)
        self.calls: list[tuple[list[str], dict[str, Any]]] = []

    def __call__(
        self,
        command: Sequence[str],
        **kwargs: Any,
    ) -> subprocess.CompletedProcess[object]:
        copied = dict(kwargs)
        self.calls.append((list(command), copied))
        status = self.statuses.pop(0) if self.statuses else 0
        return subprocess.CompletedProcess(command, status)


@pytest.mark.skipif(sys.platform == "win32", reason="POSIX PATH is case-sensitive")
def test_effective_path_preserves_case_distinct_posix_directories(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("PATH", "/opt/Agent/bin:/opt/agent/bin")

    assert effective_path().split(":") == ["/opt/Agent/bin", "/opt/agent/bin"]


def _sdk(tmp_path: Path) -> Path:
    sdk = tmp_path / "clef-sdk"
    sdk.mkdir()
    (sdk / "clef-sdk.cabal").write_text("name: clef-sdk\n", encoding="utf-8")
    return sdk


def _workspace(tmp_path: Path) -> tuple[Path, Path]:
    sdk = _sdk(tmp_path)
    root = tmp_path / "project"
    initialize_workspace(root, sdk=sdk)
    return root, sdk


def _locator(tmp_path: Path):
    tools = {
        name: str((tmp_path / "tools" / f"{name}.exe").resolve())
        for name in ("cabal", "ghc", "runghc")
    }
    return tools.get, tools


def test_init_is_idempotent_and_preserves_legacy_content(tmp_path: Path) -> None:
    sdk = _sdk(tmp_path)
    root = tmp_path / "project"
    control = root / ".tactus"
    control.mkdir(parents=True)
    legacy = control / "main_script.py"
    legacy_bytes = b"# legacy worker script\r\n"
    legacy.write_bytes(legacy_bytes)

    first = initialize_workspace(root, sdk=sdk)
    generated = (
        first.workspace.config_path,
        first.workspace.cabal_project_path,
        first.workspace.prompt_path,
    )
    before = {path: path.read_bytes() for path in generated}
    second = initialize_workspace(root, sdk=sdk)

    assert first.created == (
        ".tactus/tactus.toml",
        ".tactus/cabal.project",
        ".tactus/PROMPT.md",
    )
    assert second.created == ()
    assert second.preserved == first.created
    assert {path: path.read_bytes() for path in generated} == before
    assert legacy.read_bytes() == legacy_bytes
    assert first.workspace.scripts_path.is_dir()


def test_init_cli_accepts_root_option_and_explicit_sdk(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sdk = _sdk(tmp_path)
    root = tmp_path / "project"

    assert cli.main(["init", "--root", str(root), "--sdk", str(sdk), "--json"]) == 0
    result = json.loads(capsys.readouterr().out)

    assert result["workspace"] == str(root.resolve())
    assert result["sdk"] == str(sdk.resolve())


def test_init_cli_resolves_sdk_relative_to_the_caller(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _sdk(tmp_path)
    root = tmp_path / "nested" / "project"
    monkeypatch.chdir(tmp_path)

    assert cli.main(["init", str(root), "--sdk", "clef-sdk"]) == 0

    assert (root / ".tactus" / "cabal.project").read_text("utf-8") == (
        'packages:\n  "../../../clef-sdk"\n'
    )


def test_init_finds_sibling_sdk_by_default(tmp_path: Path) -> None:
    sdk = _sdk(tmp_path)
    root = tmp_path / "project"

    report = initialize_workspace(root)

    assert report.sdk_path == sdk.resolve()


def test_script_discovery_sorts_entries_then_warns_for_helpers(tmp_path: Path) -> None:
    root, _ = _workspace(tmp_path)
    workspace = open_workspace(root)
    nested = workspace.scripts_path / "nested"
    nested.mkdir()
    for relative in (
        "020_execute.hs",
        "010_plan.hs",
        "nested/010_also_plan.lhs",
        "nested/Support.hs",
        "free-form.hs",
        "README.md",
    ):
        path = workspace.scripts_path / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("main = pure ()\n", encoding="utf-8")

    scripts = discover_scripts(workspace)

    assert [(item.order, item.path.name) for item in scripts] == [
        (10, "010_plan.hs"),
        (10, "010_also_plan.lhs"),
        (20, "020_execute.hs"),
        (None, "free-form.hs"),
        (None, "Support.hs"),
    ]
    assert [item.warning is not None for item in scripts] == [
        False,
        False,
        False,
        True,
        True,
    ]


def test_explicit_scripts_allow_any_haskell_file(tmp_path: Path) -> None:
    root, _ = _workspace(tmp_path)
    workspace = open_workspace(root)
    outside = tmp_path / "UnnumberedHelper.lhs"
    outside.write_text("> main = pure ()\n", encoding="utf-8")

    assert explicit_scripts(workspace, [outside]) == (outside.resolve(),)


def test_check_builds_sdk_and_typechecks_with_runtime_environment(
    tmp_path: Path,
) -> None:
    root, _ = _workspace(tmp_path)
    script = root / ".tactus" / "scripts" / "010_plan.hs"
    script.write_text("main = pure ()\n", encoding="utf-8")
    locate, tools = _locator(tmp_path)
    executor = RecordingExecutor()

    status = cli.main(
        ["check", "--root", str(root)],
        locator=locate,
        command_executor=executor,
    )

    assert status == 0
    assert len(executor.calls) == 2
    build, build_options = executor.calls[0]
    check, check_options = executor.calls[1]
    project_directory = str((root / ".tactus").resolve())
    assert build == [
        tools["cabal"],
        "build",
        "--project-dir",
        project_directory,
        "lib:clef-sdk",
    ]
    assert check == [
        tools["cabal"],
        "exec",
        "--project-dir",
        project_directory,
        "--",
        "ghc",
        "-fno-code",
        "-package",
        "clef-sdk",
        f"-i{(root / '.tactus' / 'scripts').resolve()}",
        str(script.resolve()),
    ]
    for options in (build_options, check_options):
        assert options["cwd"] == root.resolve()
        assert options["check"] is False
        assert Path(options["env"]["TACTUS_RUNTIME_CONFIG"]).is_absolute()
        assert "stdin" not in options
        assert "stdout" not in options
        assert "stderr" not in options

    runtime = json.loads((root / ".tactus" / "runtime.json").read_text("utf-8"))
    assert runtime == {
        "api": "clef.runtime/v1",
        "workspace": str(root.resolve()),
        "default_provider": "codex",
        "providers": {
            "codex": {
                "command": ["tactus-provider-host", "codex"],
                "model": None,
                "effort": None,
                "options": {},
            },
            "claude-code": {
                "command": ["tactus-provider-host", "claude-code"],
                "model": None,
                "effort": None,
                "options": {},
            },
            "opencode": {
                "command": ["tactus-provider-host", "opencode"],
                "model": None,
                "effort": None,
                "options": {},
            },
        },
        "effects": {
            "workspace.paths": {
                "command": ["tactus-effect-host", "workspace-paths"],
                "options": {},
                "observe_invocations": True,
            }
        },
        "instructions": (root / ".tactus" / "PROMPT.md").read_text("utf-8"),
    }


def test_check_is_fail_fast_and_keep_going_preserves_first_exit(tmp_path: Path) -> None:
    root, _ = _workspace(tmp_path)
    scripts = root / ".tactus" / "scripts"
    for name in ("010_one.hs", "020_two.hs", "Support.hs"):
        (scripts / name).write_text("main = pure ()\n", encoding="utf-8")
    locate, _ = _locator(tmp_path)
    fail_fast = RecordingExecutor([0, 7, 0, 0])

    assert (
        cli.main(
            ["check", "--root", str(root)],
            locator=locate,
            command_executor=fail_fast,
        )
        == 7
    )
    assert len(fail_fast.calls) == 2

    keep_going = RecordingExecutor([0, 7, 0, 9])
    assert (
        cli.main(
            ["check", "--root", str(root), "--keep-going"],
            locator=locate,
            command_executor=keep_going,
        )
        == 7
    )
    assert len(keep_going.calls) == 4


def test_run_uses_only_entries_by_default_and_forwards_arguments(
    tmp_path: Path,
) -> None:
    root, _ = _workspace(tmp_path)
    scripts = root / ".tactus" / "scripts"
    entry = scripts / "010_run.hs"
    entry.write_text("main = pure ()\n", encoding="utf-8")
    (scripts / "Support.hs").write_text("module Support where\n", encoding="utf-8")
    locate, tools = _locator(tmp_path)
    executor = RecordingExecutor()

    assert (
        cli.main(
            ["run", "--root", str(root), "--", "--answer", "42"],
            locator=locate,
            command_executor=executor,
        )
        == 0
    )

    command = executor.calls[1][0]
    assert command[0] == tools["cabal"]
    assert "runghc" in command
    assert "--ghc-arg=-package=clef-sdk" in command
    assert f"--ghc-arg=-i{scripts.resolve()}" in command
    assert str(entry.resolve()) in command
    assert str((scripts / "Support.hs").resolve()) not in command
    assert command[-2:] == ["--answer", "42"]


def test_explicit_run_accepts_an_unnumbered_file(tmp_path: Path) -> None:
    root, _ = _workspace(tmp_path)
    outside = tmp_path / "manual.hs"
    outside.write_text("main = pure ()\n", encoding="utf-8")
    locate, _ = _locator(tmp_path)
    executor = RecordingExecutor()

    assert (
        cli.main(
            ["run", "--root", str(root), str(outside)],
            locator=locate,
            command_executor=executor,
        )
        == 0
    )
    assert str(outside.resolve()) in executor.calls[1][0]


def test_missing_tool_has_actionable_diagnostic(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    root, _ = _workspace(tmp_path)
    script = root / ".tactus" / "scripts" / "010_plan.hs"
    script.write_text("main = pure ()\n", encoding="utf-8")

    assert (
        cli.main(
            ["check", "--root", str(root)],
            locator=lambda _name: None,
        )
        == 2
    )
    captured = capsys.readouterr()
    assert "required tool `cabal` was not found" in captured.err
    assert "tactus doctor" in captured.err


def test_doctor_reports_workspace_and_each_tool(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    root, _ = _workspace(tmp_path)
    locate, tools = _locator(tmp_path)

    assert cli.main(["doctor", "--root", str(root), "--json"], locator=locate) == 0
    value = json.loads(capsys.readouterr().out)

    assert value["initialized"] is True
    assert value["tools"] == [
        {"available": True, "name": name, "path": tools[name]}
        for name in ("cabal", "ghc", "runghc")
    ]
