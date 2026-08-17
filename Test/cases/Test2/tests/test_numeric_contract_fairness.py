"""Fairness regression tests for the public Test2 numeric output contract."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

TEST2_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = TEST2_ROOT.parents[2]
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from reproduction.analysis import (  # noqa: E402
    EXPECTED_EXTRACTED_SHA256,
    EXPECTED_PDF_SHA256,
    build_validation_report,
)
from reproduction.verification import (  # noqa: E402
    numeric_reproduction_consistency,
)

from clef_sdk.model import (  # noqa: E402
    ArtifactKind,
    ArtifactRef,
    CheckStatus,
    SessionTask,
    VerifierSpec,
)
from clef_sdk.verification import VerificationContext  # noqa: E402

PDF = TEST2_ROOT / "Testarticle.pdf"
MARKDOWN = TEST2_ROOT / "review-work" / "Extractedmd" / "full.md"
BENCHMARK = TEST2_ROOT / "reproduction" / "benchmark-spec.json"


def _artifact(path: Path, kind: ArtifactKind) -> ArtifactRef:
    return ArtifactRef(
        uri=str(path.resolve()),
        description=path.name,
        kind=kind,
    )


def _verify(tmp_path: Path, report: dict[str, Any]):
    output = tmp_path / "validation-report.json"
    output.write_text(
        json.dumps(report, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    task = SessionTask(
        id="validate-numeric-contract",
        domain_function="paper.reproduction.validate_numerics.v1",
        inputs={
            "manuscript_pdf": _artifact(PDF, ArtifactKind.FILE),
            "manuscript_md": _artifact(MARKDOWN, ArtifactKind.TEXT),
            "benchmark_spec": _artifact(BENCHMARK, ArtifactKind.JSON),
        },
    )
    context = VerificationContext(
        task=task,
        workspace=tmp_path.resolve(),
        outputs={
            "validation_report": _artifact(output, ArtifactKind.JSON),
        },
    )
    spec = VerifierSpec(
        name="numeric_reproduction_consistency",
        parameters={
            "output": "validation_report",
            "pdf_input": "manuscript_pdf",
            "markdown_input": "manuscript_md",
        },
    )
    return numeric_reproduction_consistency(spec, context)


def _reverse_object_keys(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            key: _reverse_object_keys(item)
            for key, item in reversed(tuple(value.items()))
        }
    if isinstance(value, list):
        return [_reverse_object_keys(item) for item in value]
    return value


def _check(report: dict[str, Any], check_id: str) -> dict[str, Any]:
    return next(
        item for item in report["checks"] if item["check_id"] == check_id
    )


def _numeric_leaf_paths(
    value: Any,
    prefix: str = "",
) -> list[tuple[str, int | float]]:
    if isinstance(value, dict):
        leaves: list[tuple[str, int | float]] = []
        for key, item in value.items():
            path = f"{prefix}.{key}" if prefix else key
            leaves.extend(_numeric_leaf_paths(item, path))
        return leaves
    if isinstance(value, list):
        leaves = []
        for index, item in enumerate(value):
            leaves.extend(_numeric_leaf_paths(item, f"{prefix}[{index}]"))
        return leaves
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return [(prefix, value)]
    return []


def _live_sequence_179_shapes() -> tuple[dict[str, Any], tuple[float, float]]:
    """Recreate the affected values emitted by run sequence 179."""
    report = build_validation_report(PDF, MARKDOWN)
    report["paper"] = {
        "doi": "10.1016/j.jmps.2017.10.012",
        "title": (
            "A double perturbation method of postbuckling analysis in "
            "2D curved beams for assembly of 3D ribbon-shaped structures"
        ),
        "pdf_digest": (
            "sha256:"
            "23aa5423cc6af79247265dd75ba8af13137e2d9b76efa6b0358883b288019bd2"
        ),
        "markdown_digest": (
            "sha256:"
            "a3e368dc739dac6cf087d18cb9445eb579dd3be99e44dffb5914433beaa326cf"
        ),
    }
    for check_id, digest, media_type, source in (
        (
            "SRC-001",
            "23aa5423cc6af79247265dd75ba8af13137e2d9b76efa6b0358883b288019bd2",
            "application/pdf",
            "manuscript.pdf",
        ),
        (
            "SRC-002",
            "a3e368dc739dac6cf087d18cb9445eb579dd3be99e44dffb5914433beaa326cf",
            "text/markdown",
            "manuscript.md",
        ),
    ):
        source_check = _check(report, check_id)
        source_check["observed"] = {
            "sha256": digest,
            "media_type": media_type,
            "source": source,
        }
        source_check["expected"] = {
            "sha256": digest,
            "media_type": media_type,
            "source": "benchmark-spec evidence_output_contract.source_identity",
        }

    scope = _check(report, "SI-SCOPE-001")
    scope["observed"] = {
        "oblique_basis": {
            "recoverability": "partially_derivable",
            "anchor": "Eq. (47), Eq. (48)",
            "structure": "homogeneous basis plus forced/particular T_IV",
            "closed_forms": "BLOCKED: exact normalization is absent",
        },
        "polynomial_results": {
            "recoverability": "derivable",
            "anchor": "Section 3.2; Eq. (66)",
            "quantities": [
                "phi_(1)b(1)",
                "U_2(2)b(1)",
                "kappa_3_(1)b(1)",
            ],
        },
        "experiment_fea_details": {
            "recoverability": (
                "partially_identifiable_but_insufficient_for_replication"
            ),
            "anchor": "Section 3.3",
            "blocked": ["mesh", "sample size", "uncertainty"],
        },
        "figure_s1": {
            "recoverability": "partially_derivable",
            "anchor": "Section 3.3; Fig. S1",
            "quantitative_reproduction": "BLOCKED: SI file not used",
        },
    }
    scope["expected"] = {
        key: {
            "recoverability": value["recoverability"],
            "anchor": value["anchor"],
        }
        for key, value in scope["observed"].items()
    }

    figure8 = _check(report, "NUM-FIG8-001")
    host_observed_phi = figure8["observed"]["polynomial_completion"]["phi"][3]
    host_expected_phi = figure8["expected"]["polynomial_completion"]["phi"][3]
    point = {
        "sample_x": 0.25,
        "kappa3": 3.0399993234497464,
        "phi": 0.6421036611175154,
        "u2": -2.5847251916185923,
    }
    figure8["observed"]["polynomial_completion"] = dict(point)
    figure8["expected"]["polynomial_completion"] = dict(point)

    for check_id in (
        "LIMIT-OBLIQUE-001",
        "LIMIT-FEA-001",
        "LIMIT-EXPERIMENT-001",
    ):
        blocked = _check(report, check_id)
        blocked["observed"] = {
            "status": "BLOCKED",
            "reason": "Required publisher or raw-data inputs are unavailable.",
        }
        blocked["expected"] = {
            "status": "BLOCKED",
            "reason": "The unavailable inputs must remain explicit.",
        }
    return report, (host_observed_phi, host_expected_phi)


def test_numeric_contract_ignores_presentation_and_check_order(tmp_path: Path) -> None:
    """Presentation, key order, check order, and in-tolerance rounding are free."""
    report = build_validation_report(PDF, MARKDOWN)
    report["checks"].reverse()
    for check in report["checks"]:
        check["title"] = f"Alternative title for {check['check_id']}"
        check["interpretation"] = "Independent interpretation wording."
        check["diagnostic_note"] = {"producer": "independent-agent"}
    bmax = next(
        item for item in report["checks"] if item["check_id"] == "NUM-BMAX-001"
    )
    bmax["observed"]["sinusoidal"] = round(
        bmax["observed"]["sinusoidal"],
        5,
    )
    report["summary"]["diagnostic_note"] = "independently recomputed"
    report["summary"]["counts"]["WARN"] = 0
    report = _reverse_object_keys(report)

    result = _verify(tmp_path, report)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_numeric_contract_reports_tampered_check_field(tmp_path: Path) -> None:
    """A scientific-value change fails with an actionable field diagnostic."""
    report = build_validation_report(PDF, MARKDOWN)
    check = next(
        item for item in report["checks"] if item["check_id"] == "NUM-BMAX-001"
    )
    check["observed"]["sinusoidal"] += 0.1

    result = _verify(tmp_path, report)

    assert result.status is CheckStatus.FAILED
    assert any(
        "NUM-BMAX-001 observed.sinusoidal" in problem
        for problem in result.details["problems"]
    )


def test_numeric_contract_publishes_every_accepted_value_shape() -> None:
    """Every required path has a public type or representation declaration."""
    benchmark = json.loads(BENCHMARK.read_text(encoding="utf-8"))
    contract = benchmark["numeric_output_contract"]
    assert contract["summary_derivation"] == {
        "counted_statuses": ["PASS", "FAIL", "BLOCKED"],
        "additional_count_keys_ignored": True,
        "status_precedence": [
            {"positive_count": "FAIL", "result": "FAILED"},
            {
                "positive_count": "BLOCKED",
                "result": "PARTIAL_REPRODUCTION",
            },
            {"otherwise": "FULL_REPRODUCTION"},
        ],
        "passed": {"true_when_zero": ["FAIL"]},
        "fully_reproduced": {
            "true_when_zero": ["FAIL", "BLOCKED"],
        },
    }
    for check in contract["checks"]:
        semantics = check.get("value_semantics", {})
        default_semantic = check.get("default_value_semantic")
        group_paths: set[str] = set()
        for group in check.get("representation_groups", []):
            base = group["base_path"]
            assert (
                group["observed_expected_alignment"]
                == "same_representation_and_coordinates"
            )
            assert group["additional_properties_allowed"] is True
            group_paths.add(f"{base}.{group['coordinate_field']}")
            group_paths.update(
                f"{base}.{value_field}"
                for value_field in group["value_fields"]
            )
            assert group["accepted_representations"]
            assert {
                representation["name"]: representation["coordinate_scope"]
                for representation in group["accepted_representations"]
            } == {
                "profile": "all_sample_coordinates",
                "single_point": "interior_sample_coordinates",
            }
            assert all(
                representation["coordinate_type"] in {"array", "number"}
                and representation["value_type"] in {"array", "number"}
                for representation in group["accepted_representations"]
            )
        for semantic in (
            *semantics.values(),
            *(() if default_semantic is None else (default_semantic,)),
        ):
            assert semantic["accepted_representations"]
        required_paths = {
            *check["required_observed_paths"],
            *check["required_expected_paths"],
        }
        assert all(
            path in semantics
            or default_semantic is not None
            or path in group_paths
            for path in required_paths
        ), check["check_id"]


def test_numeric_contract_rejects_trivial_or_misaligned_single_points(
    tmp_path: Path,
) -> None:
    """Single points must be interior and align across observed/expected."""
    host = build_validation_report(PDF, MARKDOWN)
    host_figure8 = _check(host, "NUM-FIG8-001")

    endpoint_report, _ = _live_sequence_179_shapes()
    endpoint_figure8 = _check(endpoint_report, "NUM-FIG8-001")
    for field in ("observed", "expected"):
        profile = host_figure8[field]["polynomial_completion"]
        endpoint_figure8[field]["polynomial_completion"] = {
            name: profile[name][0]
            for name in ("sample_x", "kappa3", "phi", "u2")
        }
    endpoint = _verify(tmp_path, endpoint_report)
    assert endpoint.status is CheckStatus.FAILED
    assert any(
        "interior sample" in problem
        and "Eq. (69) endpoint-zero gauge" in problem
        for problem in endpoint.details["problems"]
    )

    mismatch_report, _ = _live_sequence_179_shapes()
    mismatch_figure8 = _check(mismatch_report, "NUM-FIG8-001")
    for field, index in (("observed", 1), ("expected", 3)):
        profile = host_figure8[field]["polynomial_completion"]
        mismatch_figure8[field]["polynomial_completion"] = {
            name: profile[name][index]
            for name in ("sample_x", "kappa3", "phi", "u2")
        }
    mismatch = _verify(tmp_path, mismatch_report)
    assert mismatch.status is CheckStatus.FAILED
    assert any(
        "single points must use the same public coordinate" in problem
        for problem in mismatch.details["problems"]
    )

    representation_report, _ = _live_sequence_179_shapes()
    representation_figure8 = _check(
        representation_report,
        "NUM-FIG8-001",
    )
    representation_figure8["observed"]["polynomial_completion"] = (
        host_figure8["observed"]["polynomial_completion"]
    )
    expected_profile = host_figure8["expected"]["polynomial_completion"]
    representation_figure8["expected"]["polynomial_completion"] = {
        name: expected_profile[name][3]
        for name in ("sample_x", "kappa3", "phi", "u2")
    }
    representation = _verify(tmp_path, representation_report)
    assert representation.status is CheckStatus.FAILED
    assert any(
        "must use the same public representation" in problem
        for problem in representation.details["problems"]
    )


def test_public_numeric_contract_contains_no_host_golden_values() -> None:
    """Only tolerances, representation limits, and sample coordinates are numeric."""
    contract = json.loads(BENCHMARK.read_text(encoding="utf-8"))[
        "numeric_output_contract"
    ]
    serialized = json.dumps(contract, ensure_ascii=False).casefold()
    assert EXPECTED_PDF_SHA256.casefold() not in serialized
    assert EXPECTED_EXTRACTED_SHA256.casefold() not in serialized

    for path, _value in _numeric_leaf_paths(contract):
        assert (
            (
                ".comparison." in path
                and path.endswith(".absolute")
            )
            or ".sample_coordinates[" in path
            or path.endswith((".minimum_properties", ".minimum_length"))
        ), f"unexpected public numeric leaf could leak a host golden: {path}"


def test_live_sequence_179_shapes_expose_only_the_real_phi_error(
    tmp_path: Path,
) -> None:
    """Rich shapes pass, while the non-Eq.-69 twist gauge remains rejected."""
    benchmark = json.loads(BENCHMARK.read_text(encoding="utf-8"))
    contract = benchmark["numeric_output_contract"]
    checks = {item["check_id"]: item for item in contract["checks"]}
    assert contract["source_identity_policy"] == {
        "authoritative_checks": {
            "pdf": "SRC-001",
            "markdown": "SRC-002",
        },
        "paper_object_is_presentation_only": True,
    }
    assert (
        checks["SRC-001"]["value_semantics"]["$"]["kind"]
        == "sha256_digest"
    )
    assert all(
        semantic["kind"] == "scope_present"
        for semantic in checks["SI-SCOPE-001"]["value_semantics"].values()
    )
    representations = checks["NUM-FIG8-001"]["representation_groups"][0]
    assert {
        item["name"]: item["phi_reference"]
        for item in representations["accepted_representations"]
    } == {
        "profile": "endpoint_zero_eq69",
        "single_point": "endpoint_zero_eq69",
    }

    report, host_phi = _live_sequence_179_shapes()
    rejected = _verify(tmp_path, report)

    assert rejected.status is CheckStatus.FAILED
    assert rejected.details["problems"]
    assert all(
        "NUM-FIG8-001" in problem and ".phi" in problem
        for problem in rejected.details["problems"]
    )

    figure8 = _check(report, "NUM-FIG8-001")
    figure8["observed"]["polynomial_completion"]["phi"] = host_phi[0]
    figure8["expected"]["polynomial_completion"]["phi"] = host_phi[1]
    figure8["observed"]["polynomial_completion"]["derivation_note"] = (
        "Independent equivalent diagnostic."
    )
    accepted = _verify(tmp_path, report)

    assert accepted.status is CheckStatus.PASSED, accepted.details["problems"]

    _check(report, "SRC-001")["observed"]["sha256"] = "0" * 64
    digest_tamper = _verify(tmp_path, report)

    assert digest_tamper.status is CheckStatus.FAILED
    assert any(
        "SRC-001 observed.$" in problem
        for problem in digest_tamper.details["problems"]
    )
