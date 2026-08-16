"""Round 04: document and argument structure."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_structure_review(inputs: ReviewInputs, workfolder: Path) -> SessionTask:
    return build_review_task(definition("structure"), inputs, workfolder)
