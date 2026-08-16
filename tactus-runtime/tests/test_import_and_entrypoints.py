from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
from importlib.util import find_spec
from pathlib import Path

from tactus_runtime.cli import build_parser


def _minimal_environment(source_root: Path) -> dict[str, str]:
    environment = {
        "PYTHONPATH": str(source_root),
        "PYTHONUTF8": "1",
    }
    for name in ("SYSTEMROOT", "WINDIR"):
        value = os.environ.get(name)
        if value is not None:
            environment[name] = value
    return environment


def test_package_import_has_no_worker_or_toolchain_side_effects() -> None:
    root = Path(__file__).parents[1]
    code = (
        "import sys\n"
        "import tactus_runtime\n"
        "blocked = {'sqlite3', 'subprocess', 'jupyter_client', 'webview'}\n"
        "loaded = sorted(blocked.intersection(sys.modules))\n"
        "if loaded:\n"
        "    raise SystemExit(','.join(loaded))\n"
    )

    completed = subprocess.run(
        [sys.executable, "-c", code],
        cwd=root,
        env=_minimal_environment(root / "src"),
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=10,
        check=False,
    )

    assert completed.returncode == 0, completed.stderr.decode()


def test_distribution_exposes_cli_and_reference_plugin_entries() -> None:
    root = Path(__file__).parents[1]
    with (root / "pyproject.toml").open("rb") as stream:
        project = tomllib.load(stream)["project"]

    assert project["name"] == "tactus-runtime"
    assert project["scripts"] == {
        "tactus": "tactus_runtime.cli:main",
        "tactus-provider-host": "tactus_runtime.provider_host:main",
        "tactus-effect-host": "tactus_runtime.effect_host:main",
    }
    assert "gui-scripts" not in project


def test_reference_plugin_entrypoints_override_legacy_stdio_codepage() -> None:
    root = Path(__file__).parents[1]
    environment = _minimal_environment(root / "src")
    environment["PYTHONUTF8"] = "0"
    environment["PYTHONIOENCODING"] = "cp936"
    request_id = "请求-😀"
    request = json.dumps(
        {
            "api": "agenstro.plugin/v1",
            "id": request_id,
            "method": "describe",
            "params": {},
        },
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")

    for module, argument in (
        ("tactus_runtime.provider_host", "codex"),
        ("tactus_runtime.effect_host", "workspace-paths"),
    ):
        completed = subprocess.run(
            [sys.executable, "-m", module, argument],
            cwd=root,
            env=environment,
            input=request,
            capture_output=True,
            timeout=10,
            check=False,
        )

        assert completed.returncode == 0, completed.stderr.decode(
            "utf-8", errors="replace"
        )
        [terminal] = [
            json.loads(line)
            for line in completed.stdout.decode("utf-8", errors="strict").splitlines()
        ]
        assert terminal["id"] == request_id
        assert terminal["ok"] is True


def test_editable_source_does_not_expose_frozen_worker_or_studio_modules() -> None:
    root = Path(__file__).parents[1]
    with (root / "pyproject.toml").open("rb") as stream:
        configuration = tomllib.load(stream)

    frozen = {
        "client",
        "jupyter_bridge",
        "jupyter_worker",
        "protocol",
        "script_file",
        "studio",
        "worker",
    }

    assert all(
        not (root / "src" / "tactus_runtime" / f"{name}.py").exists() for name in frozen
    )
    assert all(find_spec(f"tactus_runtime.{name}") is None for name in frozen)
    assert configuration["tool"]["hatch"]["build"]["targets"]["wheel"] == {
        "packages": ["src/tactus_runtime"]
    }
    assert set(
        configuration["tool"]["hatch"]["build"]["targets"]["sdist"]["exclude"]
    ) == {"/rewrite-report/**", "/runtime-check/**", "/rust/**"}
    assert "jupyter" not in configuration["project"]["optional-dependencies"]


def test_cli_surface_is_the_minimal_workspace_command_set() -> None:
    parser = build_parser()
    subparsers = next(action for action in parser._actions if action.dest == "command")

    assert set(subparsers.choices) == {
        "init",
        "list",
        "prompt",
        "generate",
        "check",
        "run",
        "doctor",
        "smoke",
    }
