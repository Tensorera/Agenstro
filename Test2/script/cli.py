"""Command-line interface for the manuscript-review workflow."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from clef_sdk.model import WorkflowState

from .orchestrator import run_review


SCRIPT_ROOT = Path(__file__).resolve().parent
TEST2_ROOT = SCRIPT_ROOT.parent
DEFAULT_PDF = TEST2_ROOT / "Testarticle.pdf"
DEFAULT_WORKFOLDER = TEST2_ROOT / "workfolder"
DEFAULT_ENV = TEST2_ROOT / ".env"
DEFAULT_PROFILE = SCRIPT_ROOT / "review_profile.toml"


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "MinerU OCR followed by twelve isolated Clef SDK reviews "
            "and one verified final synthesis."
        )
    )
    parser.add_argument("pdf", nargs="?", type=Path, default=DEFAULT_PDF)
    parser.add_argument(
        "--workfolder", type=Path, default=DEFAULT_WORKFOLDER
    )
    parser.add_argument("--env-file", type=Path, default=DEFAULT_ENV)
    parser.add_argument("--profile", type=Path, default=DEFAULT_PROFILE)
    parser.add_argument("--venue")
    parser.add_argument("--article-type", default="original_research")
    parser.add_argument("--review-language", default="zh-CN")
    parser.add_argument("--venue-guidelines", type=Path)
    parser.add_argument(
        "--ocr-model", choices=("vlm", "pipeline"), default="vlm"
    )
    parser.add_argument("--ocr-language", default="en")
    parser.add_argument("--ocr-timeout", type=float, default=1800.0)
    parser.add_argument("--ocr-poll-interval", type=float, default=5.0)
    ocr_mode = parser.add_mutually_exclusive_group()
    ocr_mode.add_argument(
        "--skip-ocr",
        action="store_true",
        help="require and reuse a digest-matching workfolder/Extractedmd",
    )
    ocr_mode.add_argument(
        "--force-ocr",
        action="store_true",
        help="replace only workfolder/Extractedmd after a successful new download",
    )
    parser.add_argument(
        "--plan-only",
        action="store_true",
        help="prepare/reuse OCR and compile the DAG without invoking agents",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        outcome = run_review(
            args.pdf,
            args.workfolder,
            env_file=args.env_file,
            profile_path=args.profile,
            venue=args.venue,
            article_type=args.article_type,
            review_language=args.review_language,
            venue_guidelines=args.venue_guidelines,
            ocr_model=args.ocr_model,
            ocr_language=args.ocr_language,
            ocr_timeout_seconds=args.ocr_timeout,
            ocr_poll_interval_seconds=args.ocr_poll_interval,
            skip_ocr=args.skip_ocr,
            force_ocr=args.force_ocr,
            plan_only=args.plan_only,
        )
    except Exception as error:
        print(f"ERROR {type(error).__name__}: {error}", file=sys.stderr)
        return 2

    result = outcome.result
    summary = {
        "plan_id": outcome.compiled.plan.id,
        "plan_digest": outcome.compiled.digest,
        "tasks": len(outcome.compiled.plan.tasks),
        "bindings": len(outcome.compiled.plan.bindings),
        "workfolder": str(outcome.profile.workspace.root),
        "state_root": str(outcome.profile.storage.state_root),
        "ocr_reused": outcome.extraction.reused,
        "plan_only": result is None,
        "run_id": None if result is None else result.run_id,
        "workflow_state": None if result is None else result.state.value,
        "metadata": [str(path) for path in outcome.metadata_paths],
    }
    print(json.dumps(summary, ensure_ascii=False, indent=2), flush=True)
    if result is None:
        return 0
    return 0 if result.state is WorkflowState.SUCCEEDED else 1


__all__ = ["main"]
