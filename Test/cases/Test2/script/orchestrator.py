"""End-to-end OCR, plan compilation, clef execution, and run persistence."""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path

from clef_sdk.adapters import OpenCodeAdapter
from clef_sdk.compiler import CompiledPlan, compile_plan
from clef_sdk.model import WorkflowResult
from clef_sdk.profiles import Profile
from clef_sdk.runtime import execute_plan

from .Mineru import (
    ExtractionResult,
    extract_pdf,
    prepare_manuscript,
    reuse_extraction,
)
from .Reviewfunction import build_review_registry
from .artifacts import (
    ProgressRecorder,
    assert_stable_slots_empty,
    persist_run_metadata,
    write_review_context,
)
from .runtime_profile import bind_profile
from .workflow import build_review_plan


@dataclass(frozen=True, slots=True)
class ReviewRunOutcome:
    extraction: ExtractionResult
    review_context: Path
    profile: Profile
    compiled: CompiledPlan
    result: WorkflowResult | None
    metadata_paths: tuple[Path, ...] = ()


def _has_stable_review_outputs(workfolder: Path) -> bool:
    return (
        any((workfolder / "Reviewprocess").glob("*/review.md"))
        or any((workfolder / "Reviewprocess").glob("*/findings.json"))
        or (workfolder / "Report" / "final-review-report.md").is_file()
        or (workfolder / "Report" / "decision.json").is_file()
    )


def _sanitized_agent_adapter(profile: Profile) -> OpenCodeAdapter:
    """Preserve provider configuration while excluding the MinerU credential."""

    value = profile.adapter
    environment = {
        key: item
        for key, item in os.environ.items()
        if key.casefold() != "mineru_api"
    }
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


def run_review(
    pdf_path: Path,
    workfolder: Path,
    *,
    env_file: Path,
    profile_path: Path,
    venue: str | None = None,
    article_type: str = "original_research",
    review_language: str = "zh-CN",
    venue_guidelines: Path | None = None,
    ocr_model: str = "vlm",
    ocr_language: str = "en",
    ocr_timeout_seconds: float = 1800.0,
    ocr_poll_interval_seconds: float = 5.0,
    skip_ocr: bool = False,
    force_ocr: bool = False,
    plan_only: bool = False,
) -> ReviewRunOutcome:
    """Run the requested workflow without modifying Clef SDK itself."""

    if skip_ocr and force_ocr:
        raise ValueError("skip_ocr and force_ocr are mutually exclusive")
    workfolder = workfolder.expanduser().resolve(strict=False)
    manuscript = prepare_manuscript(pdf_path, workfolder)
    if force_ocr and _has_stable_review_outputs(workfolder):
        raise FileExistsError(
            "cannot replace Extractedmd after stable review outputs were "
            "published; use a new workfolder"
        )
    extraction = (
        reuse_extraction(manuscript, workfolder)
        if skip_ocr
        else extract_pdf(
            manuscript,
            workfolder,
            env_file=env_file,
            model_version=ocr_model,
            language=ocr_language,
            is_ocr=True,
            timeout_seconds=ocr_timeout_seconds,
            poll_interval_seconds=ocr_poll_interval_seconds,
            force=force_ocr,
        )
    )
    review_context = write_review_context(
        workfolder,
        manuscript_sha256=extraction.manuscript_sha256,
        full_markdown_sha256=extraction.full_markdown_sha256,
        venue=venue,
        article_type=article_type,
        review_language=review_language,
        venue_guidelines=venue_guidelines,
    )
    profile = bind_profile(profile_path, workfolder)
    plan = build_review_plan(extraction, review_context, workfolder)
    compiled = compile_plan(plan, profile)
    if plan_only:
        return ReviewRunOutcome(
            extraction=extraction,
            review_context=review_context,
            profile=profile,
            compiled=compiled,
            result=None,
        )

    assert_stable_slots_empty(compiled.plan)
    progress = ProgressRecorder(workfolder)
    result = execute_plan(
        plan,
        profile=profile,
        adapter=_sanitized_agent_adapter(profile),
        verifier_registry=build_review_registry(),
        progress=progress,
    )
    metadata_paths = persist_run_metadata(
        workfolder,
        compiled.plan,
        result,
        plan_digest=compiled.digest,
        profile_digest=profile.digest,
    )
    return ReviewRunOutcome(
        extraction=extraction,
        review_context=review_context,
        profile=profile,
        compiled=compiled,
        result=result,
        metadata_paths=metadata_paths,
    )


__all__ = ["ReviewRunOutcome", "run_review"]
