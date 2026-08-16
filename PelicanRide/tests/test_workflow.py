# ruff: noqa: D103
"""Static contract tests for the Pelican Ride workflow."""

from __future__ import annotations

import json
import sys
from dataclasses import replace
from pathlib import Path

PELICAN_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = PELICAN_ROOT.parent
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(PELICAN_ROOT))

from clef_case.workflow import build_pelican_plan  # noqa: E402
from run import _agent_adapter  # noqa: E402

from clef_sdk.compiler import compile_plan  # noqa: E402
from clef_sdk.model import (  # noqa: E402
    ArtifactKind,
    FailurePolicy,
)
from clef_sdk.profiles import (  # noqa: E402
    ModelRoute,
    StorageConfig,
    WorkspaceConfig,
    load_profile,
)


def _profile(tmp_path: Path):
    template = load_profile(
        PELICAN_ROOT / "pelican_profile.toml",
        require_workspace=False,
        require_read_roots=False,
    )
    workspace = tmp_path / "workspace"
    state = tmp_path / "state"
    workspace.mkdir()
    state.mkdir()
    return replace(
        template,
        workspace=WorkspaceConfig(
            root=workspace,
            read_roots=(PELICAN_ROOT,),
        ),
        storage=StorageConfig(
            state_root=state,
            cache_enabled=False,
            fsync=False,
        ),
    )


def test_benchmark_is_a_strict_product_contract() -> None:
    benchmark = json.loads(
        (PELICAN_ROOT / "benchmark.json").read_text(encoding="utf-8")
    )
    assert benchmark["original_benchmark"]["prompt"] == (
        "Generate an SVG of a pelican riding a bicycle"
    )
    assert benchmark["technical_contract"]["publish_single_file"] is True
    assert benchmark["technical_contract"]["self_contained"] is True
    assert len(benchmark["acceptance"]) >= 8


def test_agent_adapter_preserves_effort_routes(tmp_path: Path) -> None:
    profile = _profile(tmp_path)
    profile = replace(
        profile,
        adapter=replace(
            profile.adapter,
            models={"xhigh": ModelRoute("provider/model", "low")},
        ),
    )

    adapter = _agent_adapter(profile)

    assert adapter.models == profile.adapter.models


def test_plan_has_parallel_compose_and_verified_delivery(tmp_path: Path) -> None:
    profile = _profile(tmp_path)
    plan = build_pelican_plan(PELICAN_ROOT, profile.workspace.root)
    compiled = compile_plan(plan, profile)

    assert len(compiled.plan.tasks) == 5
    assert compiled.plan.policies.max_concurrency == 2
    assert (
        compiled.plan.policies.failure_policy
        is FailurePolicy.SKIP_DEPENDENTS
    )
    assert {
        "compose-vector-art-direction",
        "compose-gameplay-architecture",
        "build-playable-prototype",
        "review-playability-and-visuals",
        "polish-and-package-exe",
    } == set(compiled.plan.tasks)
    delivery = compiled.plan.outputs["delivery_bundle"]
    assert delivery.source_task_id == "polish-and-package-exe"
    assert delivery.output_name == "delivery_bundle"


def test_every_task_publishes_one_verified_directory(tmp_path: Path) -> None:
    profile = _profile(tmp_path)
    plan = build_pelican_plan(PELICAN_ROOT, profile.workspace.root)

    verifier_names = {
        "design_bundle",
        "game_spec_bundle",
        "wpf_source_bundle",
        "review_bundle",
        "delivery_bundle",
    }
    seen: set[str] = set()
    for task in plan.tasks.values():
        assert len(task.outputs) == 1
        output = next(iter(task.outputs.values()))
        assert output.kind is ArtifactKind.DIRECTORY
        assert len(task.contract.verifiers) == 1
        verifier = task.contract.verifiers[0]
        assert verifier.required is True
        seen.add(verifier.name)
    assert seen == verifier_names
