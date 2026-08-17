"""Deterministic domain verifiers for review bundles and final synthesis."""

from __future__ import annotations

import json
import re
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from clef_sdk.model import (
    ArtifactRef,
    CheckResult,
    CheckStatus,
    FrozenDict,
    VerifierSpec,
)
from clef_sdk.verification import (
    VerificationContext,
    default_registry,
    uri_to_path,
)

from .definitions import REVIEW_DEFINITIONS


def _strict_json(path: Path) -> Mapping[str, Any]:
    def reject_constant(value: str) -> None:
        raise ValueError(f"non-finite JSON number: {value}")

    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    with path.open("r", encoding="utf-8") as stream:
        value = json.load(
            stream,
            parse_constant=reject_constant,
            object_pairs_hook=unique_object,
        )
    if not isinstance(value, dict):
        raise ValueError("JSON root must be an object")
    return value


def _output(
    context: VerificationContext, name: object
) -> tuple[ArtifactRef, Path]:
    if not isinstance(name, str) or not name:
        raise ValueError("output parameter must be a non-empty string")
    artifact = context.outputs.get(name)
    if artifact is None:
        raise ValueError(f"missing output: {name}")
    path = uri_to_path(artifact.uri).resolve(strict=True)
    if not path.is_relative_to(context.workspace):
        raise ValueError(f"output escapes task workspace: {path}")
    return artifact, path


def _input_path(context: VerificationContext, name: str) -> Path:
    value = context.task.inputs.get(name)
    if not isinstance(value, ArtifactRef):
        raise ValueError(f"input {name!r} is not a bound ArtifactRef")
    return uri_to_path(value.uri).resolve(strict=True)


def _result(
    name: str,
    problems: list[str],
    evidence: tuple[ArtifactRef, ...],
    required: bool,
) -> CheckResult:
    return CheckResult(
        name=name,
        status=CheckStatus.PASSED if not problems else CheckStatus.FAILED,
        message=(
            "cross-artifact consistency passed"
            if not problems
            else "; ".join(problems[:8])
        ),
        required=required,
        score=1.0 if not problems else 0.0,
        evidence=evidence,
        details=FrozenDict[Any]({"problems": problems[:100]}),
    )


def review_bundle_consistency(
    spec: VerifierSpec, context: VerificationContext
) -> CheckResult:
    report_artifact, report_path = _output(
        context, spec.parameters.get("report_output")
    )
    findings_artifact, findings_path = _output(
        context, spec.parameters.get("findings_output")
    )
    review_id = spec.parameters.get("review_id")
    dimension = spec.parameters.get("dimension")
    title = spec.parameters.get("title")
    if not isinstance(review_id, str) or not review_id:
        raise ValueError("review_id parameter must be a non-empty string")
    if not isinstance(dimension, str) or not dimension:
        raise ValueError("dimension parameter must be a non-empty string")
    if not isinstance(title, str) or not title:
        raise ValueError("title parameter must be a non-empty string")

    problems: list[str] = []
    document = _strict_json(findings_path)
    report = report_path.read_text(encoding="utf-8")
    if document.get("review_id") != review_id:
        problems.append("findings review_id does not match task")
    if document.get("dimension") != dimension:
        problems.append("findings dimension does not match task")
    if document.get("title") != title:
        problems.append("findings title does not match task")
    if title not in report:
        problems.append("report does not name its review dimension")

    issues = document.get("issues")
    if not isinstance(issues, list):
        problems.append("issues is not an array")
        issues = []
    seen: set[str] = set()
    issue_pattern = re.compile(rf"^{re.escape(review_id)}-I[0-9]{{2,}}$")
    top_references = document.get("reference_ids")
    declared_references = (
        set(top_references)
        if isinstance(top_references, list)
        and all(isinstance(value, str) for value in top_references)
        else set()
    )
    for index, issue in enumerate(issues):
        if not isinstance(issue, dict):
            problems.append(f"issues[{index}] is not an object")
            continue
        issue_id = issue.get("issue_id")
        if not isinstance(issue_id, str) or issue_pattern.fullmatch(issue_id) is None:
            problems.append(f"issues[{index}] has invalid issue_id")
            continue
        if issue_id in seen:
            problems.append(f"duplicate issue_id: {issue_id}")
        seen.add(issue_id)
        if issue_id not in report:
            problems.append(f"report omits issue_id {issue_id}")
        references = issue.get("reference_ids")
        if isinstance(references, list):
            undeclared = {
                value
                for value in references
                if isinstance(value, str) and value not in declared_references
            }
            if undeclared:
                problems.append(
                    f"{issue_id} uses undeclared references {sorted(undeclared)}"
                )
        if issue.get("basis") in {"reference", "venue_requirement"} and not references:
            problems.append(f"{issue_id} has reference-based claim without reference_ids")

    verdict = document.get("verdict")
    if verdict == "pass" and any(
        isinstance(issue, dict)
        and issue.get("severity") in {"critical", "major"}
        for issue in issues
    ):
        problems.append("pass verdict conflicts with critical/major issue")
    if verdict == "not_assessable" and not document.get("limitations"):
        problems.append("not_assessable verdict requires a stated limitation")
    return _result(
        "review_bundle_consistency",
        problems,
        (report_artifact, findings_artifact),
        spec.required,
    )


def final_review_consistency(
    spec: VerifierSpec, context: VerificationContext
) -> CheckResult:
    report_artifact, report_path = _output(
        context, spec.parameters.get("report_output")
    )
    decision_artifact, decision_path = _output(
        context, spec.parameters.get("decision_output")
    )
    decision = _strict_json(decision_path)
    report = report_path.read_text(encoding="utf-8")
    problems: list[str] = []

    upstream: dict[str, Mapping[str, Any]] = {}
    issue_ids: set[str] = set()
    for item in REVIEW_DEFINITIONS:
        document = _strict_json(
            _input_path(context, f"review_{item.review_id}_findings")
        )
        upstream[item.review_id] = document
        for issue in document.get("issues", []):
            if isinstance(issue, dict) and isinstance(issue.get("issue_id"), str):
                issue_ids.add(issue["issue_id"])

    dimensions = decision.get("dimension_reviews")
    if not isinstance(dimensions, list):
        problems.append("dimension_reviews is not an array")
        dimensions = []
    actual_ids = [
        value.get("review_id")
        for value in dimensions
        if isinstance(value, dict)
    ]
    expected_ids = [item.review_id for item in REVIEW_DEFINITIONS]
    if actual_ids != expected_ids:
        problems.append("dimension_reviews must be ordered exactly 01 through 12")
    for item, summary in zip(REVIEW_DEFINITIONS, dimensions, strict=False):
        if not isinstance(summary, dict):
            continue
        source = upstream[item.review_id]
        if summary.get("dimension") != item.slug:
            problems.append(f"{item.review_id}: dimension mismatch")
        if summary.get("verdict") != source.get("verdict"):
            problems.append(f"{item.review_id}: verdict differs from verified findings")
        top_ids = summary.get("top_issue_ids", [])
        if isinstance(top_ids, list):
            unknown = [
                value
                for value in top_ids
                if not isinstance(value, str) or value not in issue_ids
            ]
            if unknown:
                problems.append(f"{item.review_id}: unknown top_issue_ids {unknown[:3]}")
        if item.review_id not in report or item.title not in report:
            problems.append(f"final report omits dimension {item.review_id} {item.title}")

    priorities = decision.get("priority_actions")
    if not isinstance(priorities, list):
        problems.append("priority_actions is not an array")
        priorities = []
    actual_priorities = [
        action.get("priority")
        for action in priorities
        if isinstance(action, dict)
    ]
    if actual_priorities != list(range(1, len(priorities) + 1)):
        problems.append("priority values must be consecutive and ordered from 1")
    for index, action in enumerate(priorities):
        if not isinstance(action, dict):
            continue
        sources = action.get("source_issue_ids", [])
        if not isinstance(sources, list):
            continue
        for issue_id in sources:
            if not isinstance(issue_id, str) or issue_id not in issue_ids:
                problems.append(
                    f"priority action {index + 1} references unknown issue {issue_id!r}"
                )
            elif issue_id not in report:
                problems.append(f"final report omits priority issue {issue_id}")

    blocking = decision.get("blocking_issues", [])
    if isinstance(blocking, list):
        unknown = [
            value
            for value in blocking
            if not isinstance(value, str) or value not in issue_ids
        ]
        if unknown:
            problems.append(f"blocking_issues contains unknown IDs {unknown[:5]}")
    conflicts = decision.get("conflicts", [])
    if isinstance(conflicts, list):
        for index, conflict in enumerate(conflicts):
            if not isinstance(conflict, dict):
                continue
            review_ids = conflict.get("review_ids", [])
            if not isinstance(review_ids, list) or any(
                value not in expected_ids for value in review_ids
            ):
                problems.append(f"conflicts[{index}] contains unknown review_id")

    return _result(
        "final_review_consistency",
        problems,
        (report_artifact, decision_artifact),
        spec.required,
    )


def build_review_registry():
    registry = default_registry()
    registry.register("review_bundle_consistency", review_bundle_consistency)
    registry.register("final_review_consistency", final_review_consistency)
    return registry


__all__ = [
    "build_review_registry",
    "final_review_consistency",
    "review_bundle_consistency",
]
