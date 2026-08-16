"""Round 06: related-work coverage and positioning."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_related_work_review(
    inputs: ReviewInputs, workfolder: Path
) -> SessionTask:
    return build_review_task(definition("related-work"), inputs, workfolder)
