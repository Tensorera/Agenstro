"""Generate the deterministic cross-language alpha workflow fixture."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLEF_SRC = ROOT / "clef-sdk" / "src"
OUTPUT = ROOT / "fixtures" / "cross-language" / "alpha-workflow.json"


def build_fixture() -> dict[str, object]:
    """Construct the fixture through the public ``clef_sdk`` builder."""
    sys.path.insert(0, str(CLEF_SRC))
    try:
        from clef_sdk import Artifact, EffectKind, Task, Workflow, __version__
    finally:
        sys.path.pop(0)

    task = (
        Task.agent(
            "execute-script",
            "tactus.python-script.v1",
            "print('alpha vertical slice')\n",
        )
        .add_output(
            Artifact.text(
                "report",
                "Alpha execution report",
                "out/report.txt",
            )
        )
        .allow(EffectKind.CREATE, "out/report.txt")
    )
    workflow = (
        Workflow("alpha-vertical-slice")
        .add(task)
        .publish("report", "execute-script", "report")
    )
    return {
        "fixture_version": "agentro.cross-language-fixture/v1",
        "release_version": __version__,
        "protocol": {
            "api_major": 1,
            "api_minor": 0,
            "workflow_proto": "agentro.workflow.v1.WorkflowDefinition",
        },
        "products": [
            {
                "distribution": "clef-sdk",
                "python_import": "clef_sdk",
                "entry_points": [],
            },
            {
                "distribution": "tactus-runtime",
                "python_import": "tactus_runtime",
                "entry_points": ["tactus"],
            },
            {
                "distribution": "motivo-studio",
                "python_import": None,
                "entry_points": ["motivo-studio"],
            },
            {
                "distribution": "segno-flow",
                "python_import": "segno_flow",
                "entry_points": ["segno-flow", "segno-flow-ui"],
            },
        ],
        "workflow": workflow.to_dict(),
        "expected": {
            "workspace_id": "workspace-alpha",
            "tactus_output": "tactus worker output\n",
            "max_events": 16,
        },
    }


def render_fixture() -> str:
    """Return canonical checked-in JSON text."""
    return json.dumps(build_fixture(), indent=2, sort_keys=True) + "\n"


def main() -> int:
    """Write the fixture or verify that the checked-in file is current."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    rendered = render_fixture()
    if arguments.check:
        if not OUTPUT.is_file() or OUTPUT.read_text(encoding="utf-8") != rendered:
            print(f"generated integration fixture is stale: {OUTPUT}", file=sys.stderr)
            return 1
        print("generated integration fixture is current")
        return 0
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(rendered, encoding="utf-8", newline="\n")
    print(OUTPUT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
