"""Round 01: formal requirements."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_formal_review(inputs: ReviewInputs, workfolder: Path) -> SessionTask:
    return build_review_task(definition("formal"), inputs, workfolder)
