"""Frozen review dimensions and machine-readable output schemas."""

from __future__ import annotations

from dataclasses import dataclass

from clef_sdk.model import ArtifactRef


@dataclass(frozen=True, slots=True)
class ReviewInputs:
    """Direct, immutable inputs shared by all twelve review tasks."""

    manuscript_md: ArtifactRef
    manuscript_pdf: ArtifactRef
    review_context: ArtifactRef

    def as_mapping(self) -> dict[str, ArtifactRef]:
        return {
            "manuscript_md": self.manuscript_md,
            "manuscript_pdf": self.manuscript_pdf,
            "review_context": self.review_context,
        }


@dataclass(frozen=True, slots=True)
class ReviewDefinition:
    ordinal: int
    slug: str
    title: str
    prompt_file: str
    reference_files: tuple[str, ...]

    @property
    def review_id(self) -> str:
        return f"{self.ordinal:02d}"

    @property
    def task_id(self) -> str:
        return f"review-{self.review_id}-{self.slug}"

    @property
    def workspace_name(self) -> str:
        return f"{self.review_id}_{self.slug}"


REVIEW_DEFINITIONS = (
    ReviewDefinition(
        1,
        "formal",
        "形式审查",
        "01_formal.md",
        ("nature_initial_submission.md", "elsevier_peer_review.md", "icmje.md"),
    ),
    ReviewDefinition(
        2,
        "scope",
        "范围审查",
        "02_scope.md",
        ("nature_editorial_criteria.md", "elsevier_peer_review.md"),
    ),
    ReviewDefinition(
        3,
        "language",
        "语言审查",
        "03_language.md",
        ("elsevier_peer_review.md", "icmje.md"),
    ),
    ReviewDefinition(
        4,
        "structure",
        "结构审查",
        "04_structure.md",
        ("elsevier_peer_review.md", "icmje.md"),
    ),
    ReviewDefinition(
        5,
        "novelty",
        "创新性审查",
        "05_novelty.md",
        ("nature_editorial_criteria.md", "nature_peer_review.md"),
    ),
    ReviewDefinition(
        6,
        "related-work",
        "相关工作审查",
        "06_related_work.md",
        ("elsevier_peer_review.md", "icmje.md"),
    ),
    ReviewDefinition(
        7,
        "method",
        "方法审查",
        "07_method.md",
        ("elsevier_peer_review.md", "equator.md", "icmje.md"),
    ),
    ReviewDefinition(
        8,
        "correctness",
        "正确性审查",
        "08_correctness.md",
        ("nature_peer_review.md", "elsevier_peer_review.md"),
    ),
    ReviewDefinition(
        9,
        "reproducibility",
        "可复现性审查",
        "09_reproducibility.md",
        ("nature_reproducibility.md", "equator.md", "elsevier_peer_review.md"),
    ),
    ReviewDefinition(
        10,
        "results",
        "结果审查",
        "10_results.md",
        ("elsevier_peer_review.md", "icmje.md"),
    ),
    ReviewDefinition(
        11,
        "conclusion",
        "结论审查",
        "11_conclusion.md",
        ("nature_peer_review.md", "elsevier_peer_review.md"),
    ),
    ReviewDefinition(
        12,
        "ethics",
        "规范与伦理审查",
        "12_ethics.md",
        ("cope_peer_review_ethics.md", "icmje.md", "nature_policies.md"),
    ),
)

_BY_SLUG = {item.slug: item for item in REVIEW_DEFINITIONS}


def definition(slug: str) -> ReviewDefinition:
    try:
        return _BY_SLUG[slug]
    except KeyError as error:
        raise ValueError(f"unknown review dimension: {slug}") from error


_NONEMPTY_STRING = {"type": "string", "minLength": 1}
_STRING_ARRAY = {"type": "array", "items": _NONEMPTY_STRING}


def review_findings_schema(item: ReviewDefinition) -> dict[str, object]:
    """Return the strict JSON schema for one dimension's findings."""

    return {
        "type": "object",
        "additionalProperties": False,
        "required": [
            "schema_version",
            "review_id",
            "dimension",
            "title",
            "summary",
            "verdict",
            "confidence",
            "strengths",
            "issues",
            "questions_for_authors",
            "limitations",
            "reference_ids",
        ],
        "properties": {
            "schema_version": {"type": "string", "const": "1.0"},
            "review_id": {"type": "string", "const": item.review_id},
            "dimension": {"type": "string", "const": item.slug},
            "title": {"type": "string", "const": item.title},
            "summary": {"type": "string", "minLength": 50},
            "verdict": {
                "type": "string",
                "enum": [
                    "pass",
                    "minor_concerns",
                    "major_concerns",
                    "not_assessable",
                ],
            },
            "confidence": {
                "type": "string",
                "enum": ["high", "medium", "low"],
            },
            "strengths": _STRING_ARRAY,
            "issues": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": [
                        "issue_id",
                        "severity",
                        "claim",
                        "evidence",
                        "impact",
                        "required_action",
                        "basis",
                        "reference_ids",
                    ],
                    "properties": {
                        "issue_id": _NONEMPTY_STRING,
                        "severity": {
                            "type": "string",
                            "enum": [
                                "critical",
                                "major",
                                "minor",
                                "suggestion",
                            ],
                        },
                        "claim": _NONEMPTY_STRING,
                        "evidence": {
                            "type": "object",
                            "additionalProperties": False,
                            "required": ["section", "page", "anchor"],
                            "properties": {
                                "section": _NONEMPTY_STRING,
                                "page": {"type": ["integer", "null"], "minimum": 1},
                                "anchor": _NONEMPTY_STRING,
                            },
                        },
                        "impact": _NONEMPTY_STRING,
                        "required_action": _NONEMPTY_STRING,
                        "basis": {
                            "type": "string",
                            "enum": [
                                "manuscript",
                                "reference",
                                "venue_requirement",
                                "reviewer_judgment",
                            ],
                        },
                        "reference_ids": _STRING_ARRAY,
                    },
                },
            },
            "questions_for_authors": _STRING_ARRAY,
            "limitations": _STRING_ARRAY,
            "reference_ids": _STRING_ARRAY,
        },
    }


FINAL_DECISION_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "recommendation",
        "confidence",
        "executive_summary",
        "dimension_reviews",
        "priority_actions",
        "blocking_issues",
        "conflicts",
        "limitations",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "recommendation": {
            "type": "string",
            "enum": [
                "accept",
                "minor_revision",
                "major_revision",
                "reject",
                "unable_to_assess",
            ],
        },
        "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
        "executive_summary": {"type": "string", "minLength": 100},
        "dimension_reviews": {
            "type": "array",
            "minItems": 12,
            "maxItems": 12,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "review_id",
                    "dimension",
                    "verdict",
                    "rationale",
                    "top_issue_ids",
                ],
                "properties": {
                    "review_id": _NONEMPTY_STRING,
                    "dimension": _NONEMPTY_STRING,
                    "verdict": {
                        "type": "string",
                        "enum": [
                            "pass",
                            "minor_concerns",
                            "major_concerns",
                            "not_assessable",
                        ],
                    },
                    "rationale": {"type": "string", "minLength": 20},
                    "top_issue_ids": _STRING_ARRAY,
                },
            },
        },
        "priority_actions": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "priority",
                    "severity",
                    "action",
                    "source_issue_ids",
                ],
                "properties": {
                    "priority": {"type": "integer", "minimum": 1},
                    "severity": {
                        "type": "string",
                        "enum": ["critical", "major", "minor", "suggestion"],
                    },
                    "action": _NONEMPTY_STRING,
                    "source_issue_ids": {
                        "type": "array",
                        "minItems": 1,
                        "items": _NONEMPTY_STRING,
                    },
                },
            },
        },
        "blocking_issues": _STRING_ARRAY,
        "conflicts": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["topic", "review_ids", "resolution"],
                "properties": {
                    "topic": _NONEMPTY_STRING,
                    "review_ids": {
                        "type": "array",
                        "minItems": 2,
                        "items": _NONEMPTY_STRING,
                    },
                    "resolution": _NONEMPTY_STRING,
                },
            },
        },
        "limitations": _STRING_ARRAY,
    },
}


__all__ = [
    "FINAL_DECISION_SCHEMA",
    "REVIEW_DEFINITIONS",
    "ReviewDefinition",
    "ReviewInputs",
    "definition",
    "review_findings_schema",
]
