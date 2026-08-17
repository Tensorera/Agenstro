"""Round 08: internal and technical correctness."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_correctness_review(
    inputs: ReviewInputs, workfolder: Path
) -> SessionTask:
    return build_review_task(definition("correctness"), inputs, workfolder)
