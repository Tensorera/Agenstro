"""Deterministic orchestration metadata and workfolder artifact helpers."""

from __future__ import annotations

import hashlib
import json
import os
import threading
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from clef_sdk.model import RunState, WorkflowPlan, WorkflowResult


SCRIPT_ROOT = Path(__file__).resolve().parent
SOURCE_CATALOG = SCRIPT_ROOT / "Reviewfunction" / "references" / "SOURCES.json"


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (
        json.dumps(
            value,
            ensure_ascii=False,
            sort_keys=True,
            indent=2,
            allow_nan=False,
        )
        + "\n"
    ).encode("utf-8")
    temporary = path.parent / f".{path.name}.tmp-{os.getpid()}-{threading.get_ident()}"
    try:
        with temporary.open("xb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)
    finally:
        if temporary.exists():
            temporary.unlink()


def write_review_context(
    workfolder: Path,
    *,
    manuscript_sha256: str,
    full_markdown_sha256: str,
    venue: str | None,
    article_type: str,
    review_language: str,
    venue_guidelines: Path | None,
) -> Path:
    guidelines: dict[str, Any] | None = None
    if venue_guidelines is not None:
        source = venue_guidelines.expanduser().resolve(strict=True)
        if not source.is_file():
            raise ValueError(f"venue guidelines must be a file: {source}")
        if source.stat().st_size > 1_000_000:
            raise ValueError("venue guidelines exceed the 1 MB context limit")
        text = source.read_text(encoding="utf-8")
        guidelines = {
            "source_path": str(source),
            "sha256": _sha256_file(source),
            "text": text,
        }
    context = {
        "schema_version": "1.0",
        "manuscript_sha256": manuscript_sha256,
        "full_markdown_sha256": full_markdown_sha256,
        "venue": venue.strip() if venue and venue.strip() else None,
        "article_type": article_type,
        "review_language": review_language,
        "venue_guidelines": guidelines,
        "reference_catalog": {
            "path": str(SOURCE_CATALOG),
            "sha256": _sha256_file(SOURCE_CATALOG),
        },
        "evidence_policy": (
            "Unspecified venue requirements and inaccessible external sources "
            "must be reported as not assessable, never inferred."
        ),
    }
    target = workfolder / "review-context.json"
    if target.is_file():
        try:
            existing = json.loads(target.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            existing = None
        if existing == context:
            return target
        protected = any(
            path.is_file()
            for path in (
                *tuple((workfolder / "Reviewprocess").glob("*/review.md")),
                *tuple((workfolder / "Reviewprocess").glob("*/findings.json")),
                workfolder / "Report" / "final-review-report.md",
                workfolder / "Report" / "decision.json",
            )
        )
        if protected:
            raise FileExistsError(
                "review-context.json differs from a workfolder that already "
                "contains stable review outputs; use a new workfolder"
            )
    atomic_json(target, context)
    return target


def stable_output_paths(plan: WorkflowPlan) -> tuple[Path, ...]:
    values = {
        Path(spec.path).resolve(strict=False)
        for task in plan.tasks.values()
        for spec in task.outputs.values()
        if spec.path is not None
    }
    return tuple(sorted(values, key=lambda path: (str(path).casefold(), str(path))))


def assert_stable_slots_empty(plan: WorkflowPlan) -> None:
    existing = [path for path in stable_output_paths(plan) if path.exists()]
    if existing:
        sample = ", ".join(str(path) for path in existing[:5])
        raise FileExistsError(
            f"{len(existing)} stable review output slot(s) already exist; "
            f"use a new workfolder (first: {sample})"
        )


def _completed_artifacts(
    plan: WorkflowPlan, result: WorkflowResult
) -> list[dict[str, Any]]:
    artifacts: list[
        tuple[tuple[int, int, int, str, str], dict[str, Any]]
    ] = []
    for task_id, attempts in result.task_results.items():
        successful = next(
            (
                attempt
                for attempt in reversed(attempts)
                if attempt.state is RunState.SUCCEEDED
            ),
            None,
        )
        if successful is None:
            continue
        task = plan.tasks[task_id]
        stage = task.metadata.get("stage", 0)
        order = task.metadata.get("order", 0)
        output_ranks = task.metadata.get("output_ranks", {})
        for output_name, artifact in successful.outputs.items():
            rank = (
                output_ranks.get(output_name, 0)
                if isinstance(output_ranks, Mapping)
                else 0
            )
            record = {
                "task_id": task_id,
                "output_name": output_name,
                "uri": artifact.uri,
                "kind": artifact.kind.value,
                "description": artifact.description,
                "digest": artifact.digest,
                "media_type": artifact.media_type,
                "attempt": successful.attempt,
            }
            artifacts.append(
                (
                    (
                        int(stage),
                        int(order),
                        int(rank),
                        task_id.casefold(),
                        output_name.casefold(),
                    ),
                    record,
                )
            )
    return [record for _key, record in sorted(artifacts, key=lambda item: item[0])]


def persist_run_metadata(
    workfolder: Path,
    plan: WorkflowPlan,
    result: WorkflowResult,
    *,
    plan_digest: str,
    profile_digest: str,
) -> tuple[Path, Path, Path]:
    report_dir = workfolder / "Report"
    report_dir.mkdir(parents=True, exist_ok=True)
    workflow_result = report_dir / "workflow-result.json"
    manifest = report_dir / "artifact-manifest.json"
    summary = report_dir / "run-summary.json"
    atomic_json(workflow_result, result.to_dict())
    artifacts = _completed_artifacts(plan, result)
    atomic_json(
        manifest,
        {
            "schema_version": "1.0",
            "run_id": result.run_id,
            "plan_id": result.plan_id,
            "plan_digest": plan_digest,
            "artifacts": artifacts,
        },
    )
    atomic_json(
        summary,
        {
            "schema_version": "1.0",
            "run_id": result.run_id,
            "workflow_state": result.state.value,
            "plan_id": result.plan_id,
            "plan_digest": plan_digest,
            "profile_digest": profile_digest,
            "task_count": len(plan.tasks),
            "completed_artifact_count": len(artifacts),
            "skipped_tasks": list(result.skipped_tasks),
            "usage": result.usage.to_dict(),
            "workflow_result": str(workflow_result),
            "artifact_manifest": str(manifest),
        },
    )
    return workflow_result, manifest, summary


class ProgressRecorder:
    """Thread-safe JSONL progress sink under the clef run directory."""

    def __init__(self, workfolder: Path) -> None:
        self.workfolder = workfolder
        self._lock = threading.Lock()
        self.path: Path | None = None

    def __call__(self, event: dict[str, Any]) -> None:
        run_id = event.get("run_id")
        if not isinstance(run_id, str) or not run_id:
            return
        with self._lock:
            if self.path is None:
                self.path = (
                    self.workfolder
                    / "9900_run"
                    / run_id
                    / "progress.jsonl"
                )
                self.path.parent.mkdir(parents=True, exist_ok=True)
            payload = json.dumps(
                event,
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
                allow_nan=False,
            )
            with self.path.open("a", encoding="utf-8", newline="\n") as stream:
                stream.write(payload)
                stream.write("\n")
                stream.flush()


__all__ = [
    "ProgressRecorder",
    "assert_stable_slots_empty",
    "atomic_json",
    "persist_run_metadata",
    "stable_output_paths",
    "write_review_context",
]
