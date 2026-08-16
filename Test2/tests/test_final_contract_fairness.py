"""Fairness regressions for the public Test2 final-bundle contract."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

TEST2_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = TEST2_ROOT.parent
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from reproduction.content import (  # noqa: E402
    build_all_offline_content,
    json_text,
)
from reproduction.verification import (  # noqa: E402
    reproduction_bundle_consistency,
)

from clef_sdk.model import (  # noqa: E402
    ArtifactKind,
    ArtifactRef,
    CheckStatus,
    SessionTask,
    VerifierSpec,
)
from clef_sdk.verification import (  # noqa: E402
    VerificationContext,
    digest_path,
)

PDF = TEST2_ROOT / "Testarticle.pdf"
MARKDOWN = TEST2_ROOT / "review-work" / "Extractedmd" / "full.md"
BENCHMARK = TEST2_ROOT / "reproduction" / "benchmark-spec.json"

# Reproduces the claim-entry shape emitted by live run
# run-9628b67eabfd484aa824d032e1537cfa, final attempt 1. The descriptions are
# intentionally free prose; scientific authority comes from each public claim
# ID and its bound upstream evidence, not from matching canned wording.
LIVE_ATTEMPT_VALIDATED_CLAIMS = [
    (
        "SRC-001: immutable Test2 PDF identity pinned by SHA-256 "
        "23aa5423...019bd2 (exact match)"
    ),
    (
        "SRC-002: fixed MinerU markdown extraction identity pinned by "
        "SHA-256 a3e368dc...326cf (exact match)"
    ),
    (
        "SI-SCOPE-001: all four SI dependency scopes (oblique basis, "
        "polynomial results, experiment-FEA details, Fig. S1) identified"
    ),
    (
        "NUM-BMAX-001: maximum admissible shape parameter b_max reproduced "
        "(sinusoidal 0.159, polynomial 0.291, arc pi)"
    ),
    (
        "NUM-FIG5-001: Fig. 5 truncation errors reproduced "
        "(initial shape 0.906%, initial curvature 1.73%)"
    ),
    (
        "NUM-FIG8-001: Fig. 8 mode ratios and polynomial b^1 completion "
        "curves (phi/U2/kappa3) reproduced"
    ),
    (
        "NUM-PRESTRAIN-001: elastomer prestrain to applied strain "
        "conversion reproduced (66.7% to 40%)"
    ),
    (
        "NUM-FIG10-001: Fig. 10(c) inverse-design applied strains "
        "reproduced for ratios 0.1, 0.3 and 0.5"
    ),
    (
        "NUM-STRAIGHT-001: straight-beam limit coefficients reproduced "
        "(4*pi^2, pi^2/4, 2*pi)"
    ),
    (
        "SI-THEORY-POLYNOMIAL: polynomial-shape b^1-order results "
        "phi_(1)b(1), U_2(2)b(1), kappa_3_(1)b(1) derived from "
        "Eqs. (34),(52)-(55),(66)"
    ),
]


def _artifact(path: Path, kind: ArtifactKind) -> ArtifactRef:
    return ArtifactRef(
        uri=str(path.resolve()),
        description=path.name,
        kind=kind,
    )


def _verify_bundle(
    tmp_path: Path,
    validated_claims: list[str],
    *,
    mutate_theory: bool = False,
):
    content = build_all_offline_content(PDF, MARKDOWN)
    content["assessment"]["validated_claims"] = validated_claims
    if mutate_theory:
        content["theory"]["sections"][0]["status"] = "operator_identified"

    sources = {
        "evidence_ledger": tmp_path / "evidence-ledger.json",
        "theory_inference": tmp_path / "theory-inference.json",
        "methods_inference": tmp_path / "methods-inference.json",
        "validation_report": tmp_path / "validation-report.json",
        "inferred_supplement": tmp_path / "inferred-supplement.md",
        "assessment": tmp_path / "reproduction-assessment.json",
    }
    for role, key in (
        ("evidence_ledger", "evidence"),
        ("theory_inference", "theory"),
        ("methods_inference", "methods"),
        ("validation_report", "validation"),
        ("assessment", "assessment"),
    ):
        sources[role].write_text(json_text(content[key]), encoding="utf-8")
    sources["inferred_supplement"].write_text(
        content["supplement"],
        encoding="utf-8",
    )

    stable_paths = {
        "evidence_ledger": "Evidence/evidence-ledger.json",
        "theory_inference": "Inference/Theory/theory-inference.json",
        "methods_inference": "Inference/Methods/methods-inference.json",
        "validation_report": "Validation/validation-report.json",
        "inferred_supplement": "Report/inferred-supplement.md",
        "assessment": "Report/reproduction-assessment.json",
    }
    # Reversed order mirrors the fact that the public policy treats entries as
    # role-keyed records, not as a hidden positional tuple.
    manifest = {
        "schema_version": "1.0",
        "benchmark_id": "test2-blind-supplement-reproduction",
        "entries": [
            {
                "role": role,
                "path": stable_paths[role],
                "digest": digest_path(sources[role]),
                "verification": "clef_verified",
            }
            for role in reversed(tuple(stable_paths))
        ],
    }
    manifest_path = tmp_path / "artifact-manifest.json"
    manifest_path.write_text(json_text(manifest), encoding="utf-8")

    task = SessionTask(
        id="verify-final-contract",
        domain_function="paper.reproduction.synthesize.v1",
        inputs={
            "benchmark_spec": _artifact(BENCHMARK, ArtifactKind.JSON),
            "inventory_supplement_evidence_evidence_ledger": _artifact(
                sources["evidence_ledger"],
                ArtifactKind.JSON,
            ),
            "infer_theory_supplement_theory_inference": _artifact(
                sources["theory_inference"],
                ArtifactKind.JSON,
            ),
            "infer_methods_supplement_methods_inference": _artifact(
                sources["methods_inference"],
                ArtifactKind.JSON,
            ),
            "validate_paper_numerics_validation_report": _artifact(
                sources["validation_report"],
                ArtifactKind.JSON,
            ),
        },
    )
    context = VerificationContext(
        task=task,
        workspace=tmp_path.resolve(),
        outputs={
            "inferred_supplement": _artifact(
                sources["inferred_supplement"],
                ArtifactKind.TEXT,
            ),
            "assessment": _artifact(
                sources["assessment"],
                ArtifactKind.JSON,
            ),
            "artifact_manifest": _artifact(
                manifest_path,
                ArtifactKind.JSON,
            ),
        },
    )
    spec = VerifierSpec(
        name="reproduction_bundle_consistency",
        parameters={
            "supplement_output": "inferred_supplement",
            "assessment_output": "assessment",
            "manifest_output": "artifact_manifest",
        },
    )
    return reproduction_bundle_consistency(spec, context)


def _numeric_pass_ids() -> list[str]:
    content: dict[str, Any] = build_all_offline_content(PDF, MARKDOWN)
    return [
        item["check_id"]
        for item in content["validation"]["checks"]
        if item["status"] == "PASS"
    ]


def test_live_described_claims_and_evidence_backed_theory_pass(
    tmp_path: Path,
) -> None:
    """The exact live attempt shape passes without a wording repair."""
    result = _verify_bundle(
        tmp_path,
        list(LIVE_ATTEMPT_VALIDATED_CLAIMS),
    )

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_publicly_enumerated_method_claim_is_evidence_backed(
    tmp_path: Path,
) -> None:
    """A contracted methods fact can supplement the numeric PASS IDs."""
    claims = _numeric_pass_ids()
    claims.append("EXP-MATERIAL: PET material properties confirmed in Section 3.3")

    result = _verify_bundle(tmp_path, claims)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_missing_numeric_pass_and_blocked_promotion_fail(
    tmp_path: Path,
) -> None:
    """PASS coverage is complete and a BLOCKED ID cannot be promoted."""
    claims = _numeric_pass_ids()[1:]
    claims.append("LIMIT-FEA-001: treated as validated")

    result = _verify_bundle(tmp_path, claims)

    assert result.status is CheckStatus.FAILED
    problems = result.details["problems"]
    assert any("omits numeric PASS claims" in problem for problem in problems)
    assert any("promotes numeric BLOCKED claims" in problem for problem in problems)


def test_unlisted_additional_claim_fails(tmp_path: Path) -> None:
    """Unknown or scientifically unresolved extra claims stay rejected."""
    claims = _numeric_pass_ids()
    claims.append("SI-THEORY-OBLIQUE-BASIS: historical basis recovered")

    result = _verify_bundle(tmp_path, claims)

    assert result.status is CheckStatus.FAILED
    assert any(
        "unlisted validated claims" in problem
        for problem in result.details["problems"]
    )


def test_additional_claim_fails_without_contracted_upstream_evidence(
    tmp_path: Path,
) -> None:
    """An enumerated extra is accepted only when its source record qualifies."""
    claims = _numeric_pass_ids()
    claims.append("SI-THEORY-POLYNOMIAL: derived polynomial result")

    result = _verify_bundle(tmp_path, claims, mutate_theory=True)

    assert result.status is CheckStatus.FAILED
    assert any(
        "SI-THEORY-POLYNOMIAL lacks evidence status='derived'" in problem
        for problem in result.details["problems"]
    )
