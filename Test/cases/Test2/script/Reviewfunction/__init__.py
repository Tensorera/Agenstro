"""Twelve focused review task builders and the final synthesis task."""

from .definitions import REVIEW_DEFINITIONS, ReviewDefinition, ReviewInputs
from .final_report import build_final_report_task
from .formal_review import build_formal_review
from .scope_review import build_scope_review
from .language_review import build_language_review
from .structure_review import build_structure_review
from .novelty_review import build_novelty_review
from .related_work_review import build_related_work_review
from .method_review import build_method_review
from .correctness_review import build_correctness_review
from .reproducibility_review import build_reproducibility_review
from .results_review import build_results_review
from .conclusion_review import build_conclusion_review
from .ethics_review import build_ethics_review
from .verification import build_review_registry

REVIEW_BUILDERS = (
    build_formal_review,
    build_scope_review,
    build_language_review,
    build_structure_review,
    build_novelty_review,
    build_related_work_review,
    build_method_review,
    build_correctness_review,
    build_reproducibility_review,
    build_results_review,
    build_conclusion_review,
    build_ethics_review,
)

__all__ = [
    "REVIEW_BUILDERS",
    "REVIEW_DEFINITIONS",
    "ReviewDefinition",
    "ReviewInputs",
    "build_final_report_task",
    "build_review_registry",
]
