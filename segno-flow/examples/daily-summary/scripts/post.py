"""Publish artifacts and retain useful diagnostics even after a failed main stage."""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

workspace = Path(os.environ["SEGNO_RUN_WORKSPACE"])
artifacts = Path(os.environ["SEGNO_ARTIFACTS_DIR"])
artifacts.mkdir(parents=True, exist_ok=True)

output = workspace / "output"
if output.is_dir():
    for source in output.iterdir():
        if source.is_file() and not source.is_symlink():
            shutil.copy2(source, artifacts / source.name)

summary = {
    "run_id": os.environ["SEGNO_RUN_ID"],
    "pre_status": os.environ["SEGNO_PRE_STATUS"],
    "main_status": os.environ["SEGNO_MAIN_STATUS"],
}
(artifacts / "run-summary.json").write_text(
    json.dumps(summary, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)

if summary["main_status"] == "succeeded":
    shutil.rmtree(workspace, ignore_errors=True)
    print("Artifacts published; successful temporary workspace removed.")
else:
    print("Prior stage failed; temporary workspace retained for diagnosis.")
