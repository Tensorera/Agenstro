"""Compile and execute the Test2 reproduction DAG offline or with an agent."""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import replace
from pathlib import Path

if __package__ in {None, ""}:
    TEST2_ROOT_FOR_IMPORT = Path(__file__).resolve().parents[1]
    REPOSITORY_ROOT_FOR_IMPORT = TEST2_ROOT_FOR_IMPORT.parents[2]
    sys.path.insert(0, str(REPOSITORY_ROOT_FOR_IMPORT / "clef-sdk" / "src"))
    sys.path.insert(0, str(TEST2_ROOT_FOR_IMPORT))

from script.runtime_profile import bind_profile

from clef_sdk.adapters import FakeAdapter, OpenCodeAdapter
from clef_sdk.compiler import compile_plan
from clef_sdk.model import WorkflowState
from clef_sdk.profiles import WorkspaceConfig
from clef_sdk.runtime import ConsoleProgressObserver, execute_plan
from reproduction.offline import make_offline_callback
from reproduction.verification import build_reproduction_registry
from reproduction.workflow import (
    build_reproduction_plan,
    prepare_blind_input_bundle,
)

HERE = Path(__file__).resolve().parent
TEST2_ROOT = HERE.parent
DEFAULT_PROFILE = HERE / "reproduction_profile.toml"
_AGENT_ENVIRONMENT_ALLOWLIST = frozenset(
    {
        "appdata",
        "comspec",
        "homedrive",
        "homepath",
        "localappdata",
        "path",
        "pathext",
        "systemroot",
        "temp",
        "tmp",
        "userprofile",
        "windir",
        "xdg_config_home",
    }
)
_AGENT_STABLE_READ_DIRECTORIES = (
    "Evidence",
    "Inference/Theory",
    "Inference/Methods",
    "Validation",
    "Report",
)


def _atomic_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def _agent_adapter(profile) -> OpenCodeAdapter:
    value = profile.adapter
    environment = {
        key: item
        for key, item in os.environ.items()
        if key.casefold() in _AGENT_ENVIRONMENT_ALLOWLIST
    }
    allowed_external_roots = (
        *profile.workspace.read_roots,
        *(
            profile.workspace.root / relative
            for relative in _AGENT_STABLE_READ_DIRECTORIES
        ),
    )
    external_directory = {"*": "deny"}
    for root in allowed_external_roots:
        normalized = root.resolve(strict=False).as_posix().rstrip("/")
        external_directory[normalized] = "allow"
        external_directory[f"{normalized}/**"] = "allow"
    environment["OPENCODE_CONFIG_CONTENT"] = json.dumps(
        {
            "$schema": "https://opencode.ai/config.json",
            "share": "disabled",
            "permission": {
                "external_directory": external_directory,
                "webfetch": "deny",
                "websearch": "deny",
                "task": "deny",
            },
        },
        ensure_ascii=True,
        separators=(",", ":"),
    )
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


def bind_reproduction_profile(profile_path: Path, workfolder: Path):
    """Bind state/workspace to the run-local blind input allowlist."""

    profile = bind_profile(profile_path, workfolder)
    blind_inputs = prepare_blind_input_bundle(TEST2_ROOT, workfolder)
    profile = replace(
        profile,
        workspace=WorkspaceConfig(
            root=workfolder,
            read_roots=(blind_inputs,),
        ),
    )
    profile.validate_filesystem()
    return profile


def _stable_outputs(workfolder: Path) -> tuple[Path, ...]:
    return (
        workfolder / "Evidence" / "evidence-report.md",
        workfolder / "Evidence" / "evidence-ledger.json",
        workfolder / "Inference" / "Theory" / "theory-inference.md",
        workfolder / "Inference" / "Theory" / "theory-inference.json",
        workfolder / "Inference" / "Methods" / "methods-inference.md",
        workfolder / "Inference" / "Methods" / "methods-inference.json",
        workfolder / "Validation" / "validation-report.md",
        workfolder / "Validation" / "validation-report.json",
        workfolder / "Report" / "inferred-supplement.md",
        workfolder / "Report" / "reproduction-assessment.json",
        workfolder / "Report" / "artifact-manifest.json",
    )


def build_parser() -> argparse.ArgumentParser:
    """Return the runner CLI parser."""

    parser = argparse.ArgumentParser(
        description=("Run the blind Test2 supplementary reconstruction Clef DAG.")
    )
    parser.add_argument(
        "--mode",
        choices=("offline", "agent"),
        default="offline",
        help="FakeAdapter framework regression or real OpenCode agent run.",
    )
    parser.add_argument(
        "--workfolder",
        type=Path,
        default=HERE / "output",
    )
    parser.add_argument(
        "--profile",
        type=Path,
        default=DEFAULT_PROFILE,
    )
    parser.add_argument(
        "--plan-only",
        action="store_true",
        help="Compile the five-node plan without executing it.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Compile, execute, verify, publish and persist the reproduction run."""

    args = build_parser().parse_args(argv)
    workfolder = args.workfolder.expanduser().resolve(strict=False)
    workfolder.mkdir(parents=True, exist_ok=True)
    profile = bind_reproduction_profile(args.profile, workfolder)
    plan = build_reproduction_plan(TEST2_ROOT, workfolder)
    compiled = compile_plan(plan, profile)
    if args.plan_only:
        print(
            json.dumps(
                {
                    "plan_id": compiled.plan.id,
                    "plan_digest": compiled.digest,
                    "tasks": list(compiled.plan.tasks),
                    "bindings": len(compiled.plan.bindings),
                },
                ensure_ascii=False,
                indent=2,
            )
        )
        return 0

    occupied = [path for path in _stable_outputs(workfolder) if path.exists()]
    if occupied:
        raise FileExistsError(
            "stable reproduction outputs already exist; use a new "
            f"--workfolder (first conflict: {occupied[0]})"
        )

    if args.mode == "offline":
        callback = make_offline_callback(
            TEST2_ROOT / "Testarticle.pdf",
            TEST2_ROOT / "review-work" / "Extractedmd" / "full.md",
        )
        adapter = FakeAdapter([callback] * len(compiled.plan.tasks))
    else:
        adapter = _agent_adapter(profile)
    result = execute_plan(
        compiled.plan,
        profile=profile,
        adapter=adapter,
        verifier_registry=build_reproduction_registry(),
        observer=ConsoleProgressObserver(stream=sys.stderr),
    )
    run_directory = workfolder / "9900_run" / result.run_id
    _atomic_json(run_directory / "workflow-result.json", result.to_dict())
    summary = {
        "mode": args.mode,
        "plan_id": compiled.plan.id,
        "plan_digest": compiled.digest,
        "run_id": result.run_id,
        "workflow_state": result.state.value,
        "task_states": {
            task_id: attempts[-1].state.value
            for task_id, attempts in result.task_results.items()
            if attempts
        },
        "outputs": {name: artifact.uri for name, artifact in result.outputs.items()},
        "execution_summary": (
            None if result.summary is None else result.summary.to_dict()
        ),
    }
    _atomic_json(run_directory / "run-summary.json", summary)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if result.state is WorkflowState.SUCCEEDED else 1


if __name__ == "__main__":
    raise SystemExit(main())
