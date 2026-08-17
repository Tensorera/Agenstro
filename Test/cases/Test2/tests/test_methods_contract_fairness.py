"""Semantic-equivalence tests for the public Test2 methods contract."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

TEST2_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = TEST2_ROOT.parents[2]
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from reproduction.content import (  # noqa: E402
    build_methods_inference,
    render_methods_report,
)
from reproduction.verification import methods_inference_consistency  # noqa: E402

from clef_sdk.model import (  # noqa: E402
    ArtifactKind,
    ArtifactRef,
    CheckStatus,
    SessionTask,
    VerifierSpec,
)
from clef_sdk.verification import VerificationContext  # noqa: E402

BENCHMARK = TEST2_ROOT / "reproduction" / "benchmark-spec.json"


def _artifact(path: Path, kind: ArtifactKind) -> ArtifactRef:
    return ArtifactRef(
        uri=str(path.resolve()),
        description=path.name,
        kind=kind,
    )


def _verify(tmp_path: Path, methods: dict[str, Any]):
    json_path = tmp_path / "methods-inference.json"
    report_path = tmp_path / "methods-inference.md"
    json_path.write_text(
        json.dumps(methods, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    report_path.write_text(render_methods_report(methods), encoding="utf-8")
    task = SessionTask(
        id="verify-methods-contract",
        domain_function="paper.reproduction.infer_methods.v1",
        inputs={
            "benchmark_spec": _artifact(BENCHMARK, ArtifactKind.JSON),
        },
    )
    context = VerificationContext(
        task=task,
        workspace=tmp_path.resolve(),
        outputs={
            "methods_inference": _artifact(json_path, ArtifactKind.JSON),
            "methods_report": _artifact(report_path, ArtifactKind.TEXT),
        },
    )
    spec = VerifierSpec(
        name="methods_inference_consistency",
        parameters={
            "json_output": "methods_inference",
            "report_output": "methods_report",
        },
    )
    return methods_inference_consistency(spec, context)


def _fact(methods: dict[str, Any], fact_id: str) -> dict[str, Any]:
    return next(
        item for item in methods["confirmed_facts"] if item["fact_id"] == fact_id
    )


def test_equivalent_scientific_typography_and_spacing_pass(tmp_path: Path) -> None:
    """Equivalent Unicode symbols and spacing must not trigger a repair."""
    methods = build_methods_inference()
    _fact(methods, "EXP-MATERIAL")["value"] = (
        "PET, thickness 40 \N{MICRO SIGN}m, E = 3.5 GPa, "
        "\N{GREEK SMALL LETTER NU} = 0.39"
    )
    _fact(methods, "EXP-GEOMETRY")["value"] = "h : w : L_S = 1 : 30 : 900"
    _fact(methods, "EXP-SHAPE-B")["value"] = (
        "b = 0.15, 0.25, 2π/3 for sinusoidal, polynomial, arc"
    )

    result = _verify(tmp_path, methods)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_changed_method_quantity_still_fails(tmp_path: Path) -> None:
    """A changed scientific quantity must still fail semantic verification."""
    methods = build_methods_inference()
    _fact(methods, "EXP-MATERIAL")["value"] = (
        "PET, thickness 41 \N{MICRO SIGN}m, E = 3.5 GPa, "
        "\N{GREEK SMALL LETTER NU} = 0.39"
    )

    result = _verify(tmp_path, methods)

    assert result.status is CheckStatus.FAILED
    assert any(
        "EXP-MATERIAL value" in problem for problem in result.details["problems"]
    )
