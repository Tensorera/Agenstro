"""Round 09: reproducibility and reporting completeness."""

from pathlib import Path

from clef_sdk.model import SessionTask

from .definitions import ReviewInputs, definition
from .factory import build_review_task


def build_reproducibility_review(
    inputs: ReviewInputs, workfolder: Path
) -> SessionTask:
    return build_review_task(definition("reproducibility"), inputs, workfolder)
