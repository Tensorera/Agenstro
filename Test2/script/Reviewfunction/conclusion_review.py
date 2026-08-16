"""Round 11: conclusions and claim support."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_conclusion_review(
    inputs: ReviewInputs, workfolder: Path
) -> SessionTask:
    return build_review_task(definition("conclusion"), inputs, workfolder)
