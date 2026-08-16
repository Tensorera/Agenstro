"""Build the example with Segno's strict offline package authoring API."""

from __future__ import annotations

import sys
from pathlib import Path

from segno_flow import build_task_package

source_root = Path(__file__).resolve().parent
destination = (
    Path(sys.argv[1]).expanduser().resolve()
    if len(sys.argv) > 1
    else source_root / "dist" / "daily-summary.zip"
)
result = build_task_package(source_root, destination)
print(result.path)
