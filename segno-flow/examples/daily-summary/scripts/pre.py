"""Collect immutable input material into this run's temporary workspace."""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

workspace = Path(os.environ["SEGNO_RUN_WORKSPACE"])
working_directory = Path(os.environ["SEGNO_WORKING_DIRECTORY"])
material_directory = workspace / "materials"
material_directory.mkdir(parents=True, exist_ok=True)

inbox = working_directory / "inbox"
inbox.mkdir(parents=True, exist_ok=True)
collected: list[str] = []
for source in sorted(inbox.iterdir()):
    if source.is_file() and not source.is_symlink():
        shutil.copy2(source, material_directory / source.name)
        collected.append(source.name)

(workspace / "input-manifest.json").write_text(
    json.dumps({"files": collected}, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
print(f"Collected {len(collected)} input file(s).")
