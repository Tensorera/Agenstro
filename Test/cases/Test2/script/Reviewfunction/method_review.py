"""Round 07: research design and methods."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_method_review(inputs: ReviewInputs, workfolder: Path) -> SessionTask:
    return build_review_task(definition("method"), inputs, workfolder)
