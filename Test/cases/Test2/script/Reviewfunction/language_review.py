"""Round 03: language and clarity."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_language_review(inputs: ReviewInputs, workfolder: Path) -> SessionTask:
    return build_review_task(definition("language"), inputs, workfolder)
