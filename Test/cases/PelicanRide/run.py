# ruff: noqa: E402
"""Compile and execute the Pelican Ride Clef SDK case study."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from dataclasses import replace
from datetime import UTC, datetime
from pathlib import Path

CASE_ROOT = Path(__file__).resolve().parent
REPOSITORY_ROOT = CASE_ROOT.parents[2]
if str(REPOSITORY_ROOT / "clef-sdk" / "src") not in sys.path:
    sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
if str(CASE_ROOT) not in sys.path:
    sys.path.insert(0, str(CASE_ROOT))

from clef_case.verification import build_pelican_registry
from clef_case.workflow import build_pelican_plan

from clef_sdk.adapters import OpenCodeAdapter
from clef_sdk.compiler import compile_plan
from clef_sdk.model import WorkflowState
from clef_sdk.profiles import (
    StorageConfig,
    WorkspaceConfig,
    load_profile,
)
from clef_sdk.runtime import ConsoleProgressObserver, execute_plan

DEFAULT_PROFILE = CASE_ROOT / "pelican_profile.toml"
_ENVIRONMENT_ALLOWLIST = frozenset(
    {
        "appdata",
        "comspec",
        "dotnet_root",
        "homedrive",
        "homepath",
        "localappdata",
        "nuget_packages",
        "path",
        "pathext",
        "programdata",
        "programfiles",
        "programfiles(x86)",
        "systemdrive",
        "systemroot",
        "temp",
        "tmp",
        "userprofile",
        "windir",
        "xdg_config_home",
    }
)


def _utc_now() -> str:
    return datetime.now(UTC).isoformat().replace("+00:00", "Z")


def _atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def _bind_profile(profile_path: Path, workfolder: Path):
    template = load_profile(
        profile_path.expanduser().resolve(strict=True),
        require_workspace=False,
        require_read_roots=False,
    )
    identity = hashlib.sha256(
        str(workfolder).casefold().encode("utf-8")
    ).hexdigest()[:16]
    state_root = (
        CASE_ROOT / ".clef-state" / f"{workfolder.name}-{identity}"
    ).resolve(strict=False)
    state_root.mkdir(parents=True, exist_ok=True)
    profile = replace(
        template,
        workspace=WorkspaceConfig(
            root=workfolder,
            read_roots=(CASE_ROOT,),
        ),
        storage=StorageConfig(
            state_root=state_root,
            cas_dir=template.storage.cas_dir,
            traces_dir=template.storage.traces_dir,
            cache_dir=template.storage.cache_dir,
            manifests_dir=template.storage.manifests_dir,
            cache_enabled=template.storage.cache_enabled,
            fsync=template.storage.fsync,
        ),
    )
    profile.validate_filesystem()
    return profile


def _agent_adapter(profile) -> OpenCodeAdapter:
    value = profile.adapter
    environment = {
        key: item
        for key, item in os.environ.items()
        if key.casefold() in _ENVIRONMENT_ALLOWLIST
    }
    # This is an intentionally permissive creative/build case. Clef still
    # audits declared workspace effects and verifies every published artifact.
    environment["OPENCODE_CONFIG_CONTENT"] = json.dumps(
        {
            "$schema": "https://opencode.ai/config.json",
            "share": "disabled",
            "permission": "allow",
        },
        ensure_ascii=True,
        separators=(",", ":"),
    )
    environment["DOTNET_CLI_TELEMETRY_OPTOUT"] = "1"
    environment["DOTNET_NOLOGO"] = "1"
    environment["PYTHONUNBUFFERED"] = "1"
    return OpenCodeAdapter(
        executable=value.executable,
        model=value.model,
        agent=value.agent,
        variant=value.variant,
        attach_url=value.attach_url,
        auto_approve=value.auto_approve,
        pure=value.pure,
        inherit_environment=False,
        extra_args=value.extra_args,
        environment=environment,
        models=value.models,
    )


def build_parser() -> argparse.ArgumentParser:
    """Create the command-line parser."""
    parser = argparse.ArgumentParser(
        description=(
            "Run the agentic Pelican Ride design/build/review/package workflow."
        )
    )
    parser.add_argument(
        "--workfolder",
        type=Path,
        default=CASE_ROOT / "runs" / "output",
    )
    parser.add_argument(
        "--profile",
        type=Path,
        default=DEFAULT_PROFILE,
    )
    parser.add_argument(
        "--plan-only",
        action="store_true",
        help="Compile and print the five-node plan without executing agents.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Compile, execute, verify and persist one Pelican Ride run."""
    args = build_parser().parse_args(argv)
    workfolder = args.workfolder.expanduser().resolve(strict=False)
    workfolder.mkdir(parents=True, exist_ok=True)
    profile = _bind_profile(args.profile, workfolder)
    plan = build_pelican_plan(CASE_ROOT, workfolder)
    compiled = compile_plan(plan, profile)
    if args.plan_only:
        print(
            json.dumps(
                {
                    "plan_id": compiled.plan.id,
                    "plan_digest": compiled.digest,
                    "tasks": list(compiled.plan.tasks),
                    "bindings": len(compiled.plan.bindings),
                    "output": "delivery_bundle",
                },
                ensure_ascii=False,
                indent=2,
            )
        )
        return 0

    final_directory = workfolder / "0400_delivery" / "delivery-bundle"
    if final_directory.exists():
        raise FileExistsError(
            "final delivery already exists; choose a new --workfolder: "
            f"{final_directory}"
        )

    status_path = workfolder / "run-status.json"
    _atomic_json(
        status_path,
        {
            "schema_version": "1.0",
            "state": "STARTING",
            "started_at_utc": _utc_now(),
            "plan_id": compiled.plan.id,
            "plan_digest": compiled.digest,
            "workfolder": str(workfolder),
            "state_root": str(profile.storage.state_root),
            "expected_exe": str(final_directory / "PelicanRide.exe"),
        },
    )
    try:
        _atomic_json(
            status_path,
            {
                "schema_version": "1.0",
                "state": "RUNNING",
                "started_at_utc": _utc_now(),
                "plan_id": compiled.plan.id,
                "plan_digest": compiled.digest,
                "workfolder": str(workfolder),
                "state_root": str(profile.storage.state_root),
                "expected_exe": str(final_directory / "PelicanRide.exe"),
            },
        )
        result = execute_plan(
            compiled.plan,
            profile=profile,
            adapter=_agent_adapter(profile),
            verifier_registry=build_pelican_registry(),
            observer=ConsoleProgressObserver(stream=sys.stderr),
        )
    except BaseException as exc:
        _atomic_json(
            status_path,
            {
                "schema_version": "1.0",
                "state": "INTERRUPTED",
                "completed_at_utc": _utc_now(),
                "plan_id": compiled.plan.id,
                "plan_digest": compiled.digest,
                "workfolder": str(workfolder),
                "state_root": str(profile.storage.state_root),
                "error_type": type(exc).__name__,
                "error": str(exc),
                "expected_exe": str(final_directory / "PelicanRide.exe"),
            },
        )
        raise

    run_directory = workfolder / "9900_run" / result.run_id
    summary = {
        "schema_version": "1.0",
        "plan_id": compiled.plan.id,
        "plan_digest": compiled.digest,
        "run_id": result.run_id,
        "workflow_state": result.state.value,
        "started_at_utc": (
            None if result.summary is None else result.summary.started_at
        ),
        "completed_at_utc": (
            None if result.summary is None else result.summary.completed_at
        ),
        "task_states": {
            task_id: attempts[-1].state.value
            for task_id, attempts in result.task_results.items()
            if attempts
        },
        "outputs": {
            name: artifact.uri for name, artifact in result.outputs.items()
        },
        "execution_summary": (
            None if result.summary is None else result.summary.to_dict()
        ),
        "exe": (
            str(final_directory / "PelicanRide.exe")
            if (final_directory / "PelicanRide.exe").is_file()
            else None
        ),
    }
    _atomic_json(run_directory / "workflow-result.json", result.to_dict())
    _atomic_json(run_directory / "run-summary.json", summary)
    _atomic_json(
        status_path,
        {
            **summary,
            "state": result.state.value,
            "workfolder": str(workfolder),
            "state_root": str(profile.storage.state_root),
        },
    )
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if result.state is WorkflowState.SUCCEEDED else 1


if __name__ == "__main__":
    raise SystemExit(main())
