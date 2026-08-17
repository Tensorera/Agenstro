#!/usr/bin/env python3
"""Small executable seam for the complete review orchestration."""

from __future__ import annotations

import sys
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parent
TEST2_ROOT = SCRIPT_ROOT.parent
REPOSITORY_ROOT = TEST2_ROOT.parents[2]
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from script.cli import main  # noqa: E402


if __name__ == "__main__":
    raise SystemExit(main())
