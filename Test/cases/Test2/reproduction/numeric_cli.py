"""CLI for the deterministic Test2 numerical reproduction checks."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

if __package__ in {None, ""}:
    import sys

    TEST2_ROOT_FOR_IMPORT = Path(__file__).resolve().parents[1]
    REPOSITORY_ROOT_FOR_IMPORT = TEST2_ROOT_FOR_IMPORT.parents[2]
    sys.path.insert(0, str(REPOSITORY_ROOT_FOR_IMPORT / "clef-sdk" / "src"))
    sys.path.insert(0, str(TEST2_ROOT_FOR_IMPORT))
    from reproduction.analysis import (  # type: ignore[import-not-found]
        build_validation_report,
        render_validation_markdown,
    )
else:
    from .analysis import build_validation_report, render_validation_markdown


def _atomic_text(path: Path, content: str) -> None:
    path = path.expanduser().resolve(strict=False)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(content, encoding="utf-8")
    temporary.replace(path)


def build_parser() -> argparse.ArgumentParser:
    """Return the CLI parser."""

    parser = argparse.ArgumentParser(
        description="Recompute Test2 claims without the publisher SI."
    )
    parser.add_argument("--pdf", required=True, type=Path)
    parser.add_argument("--markdown", required=True, type=Path)
    parser.add_argument("--json-out", required=True, type=Path)
    parser.add_argument("--markdown-out", required=True, type=Path)
    return parser


def main(argv: list[str] | None = None) -> int:
    """Run checks, write both reports and fail on a numerical mismatch."""

    args = build_parser().parse_args(argv)
    report = build_validation_report(args.pdf, args.markdown)
    _atomic_text(
        args.json_out,
        json.dumps(
            report,
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    _atomic_text(args.markdown_out, render_validation_markdown(report))
    return 0 if report["summary"]["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
