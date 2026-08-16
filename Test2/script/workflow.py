"""Build the 12-way fan-out and one-way final fan-in review DAG."""

from __future__ import annotations

import hashlib
from pathlib import Path

from clef_sdk.model import (
    ArtifactBinding,
    ArtifactKind,
    ArtifactRef,
    FailurePolicy,
    FrozenDict,
    SessionTask,
    WorkflowPlan,
    WorkflowPolicies,
)

from .Mineru import ExtractionResult
from .Reviewfunction import (
    REVIEW_BUILDERS,
    ReviewInputs,
    build_final_report_task,
)


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build_review_inputs(
    extraction: ExtractionResult,
    review_context: Path,
) -> ReviewInputs:
    review_context = review_context.resolve(strict=True)
    return ReviewInputs(
        manuscript_md=ArtifactRef(
            uri=str(extraction.full_markdown.resolve(strict=True)),
            description="MinerU full.md manuscript text",
            kind=ArtifactKind.TEXT,
            digest=f"sha256:{extraction.full_markdown_sha256}",
            media_type="text/markdown",
        ),
        manuscript_pdf=ArtifactRef(
            uri=str(extraction.manuscript_pdf.resolve(strict=True)),
            description="immutable original manuscript PDF",
            kind=ArtifactKind.FILE,
            digest=f"sha256:{extraction.manuscript_sha256}",
            media_type="application/pdf",
        ),
        review_context=ArtifactRef(
            uri=str(review_context),
            description="venue and review invocation context",
            kind=ArtifactKind.JSON,
            digest=f"sha256:{_sha256_file(review_context)}",
            media_type="application/json",
        ),
    )


def build_review_plan(
    extraction: ExtractionResult,
    review_context: Path,
    workfolder: Path,
) -> WorkflowPlan:
    workfolder = workfolder.expanduser().resolve(strict=True)
    direct_inputs = build_review_inputs(extraction, review_context)
    review_tasks = {
        task.id: task
        for task in (
            builder(direct_inputs, workfolder)
            for builder in REVIEW_BUILDERS
        )
    }
    if len(review_tasks) != 12:
        raise ValueError("review workflow must contain exactly 12 focused tasks")
    final_task, bindings = build_final_report_task(
        review_tasks, direct_inputs, workfolder
    )
    tasks = dict(review_tasks)
    tasks[final_task.id] = final_task
    identity = hashlib.sha256(
        (
            extraction.manuscript_sha256
            + extraction.full_markdown_sha256
            + _sha256_file(review_context)
        ).encode("ascii")
    ).hexdigest()[:20]
    return WorkflowPlan(
        id=f"manuscript-review-{identity}",
        tasks=FrozenDict[SessionTask](tasks),
        bindings=bindings,
        policies=WorkflowPolicies(
            max_concurrency=4,
            failure_policy=FailurePolicy.SKIP_DEPENDENTS,
            fail_fast=False,
            max_subagent_depth=1,
            max_fan_out=32,
        ),
        outputs=FrozenDict[ArtifactBinding](
            {
                "final_report": ArtifactBinding(
                    source_task_id=final_task.id,
                    output_name="final_report",
                ),
                "decision": ArtifactBinding(
                    source_task_id=final_task.id,
                    output_name="decision",
                ),
            }
        ),
    )


__all__ = ["build_review_inputs", "build_review_plan"]
