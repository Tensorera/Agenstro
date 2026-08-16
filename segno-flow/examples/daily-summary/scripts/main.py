"""Produce the example report entirely inside the run workspace."""

from __future__ import annotations

import os
from pathlib import Path

from helpers import describe_files

workspace = Path(os.environ["SEGNO_RUN_WORKSPACE"])
output = workspace / "output"
output.mkdir(parents=True, exist_ok=True)
lines = ["# Daily summary", "", *describe_files(workspace / "materials"), ""]
(output / "summary.md").write_text("\n".join(lines), encoding="utf-8")
print("Report generated in the temporary workspace.")
