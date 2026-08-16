"""Strict output schemas for the Test2 reproduction DAG."""

from __future__ import annotations

NONEMPTY = {"type": "string", "minLength": 1}
STRING_ARRAY = {"type": "array", "items": NONEMPTY}
DIGEST = {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}


EVIDENCE_LEDGER_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "benchmark_id",
        "source_identity",
        "policy",
        "si_dependencies",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "benchmark_id": {
            "type": "string",
            "const": "test2-blind-supplement-reproduction",
        },
        "source_identity": {
            "type": "object",
            "additionalProperties": False,
            "required": ["pdf_digest", "markdown_digest", "doi"],
            "properties": {
                "pdf_digest": DIGEST,
                "markdown_digest": DIGEST,
                "doi": {
                    "type": "string",
                    "const": "10.1016/j.jmps.2017.10.012",
                },
            },
        },
        "policy": {
            "type": "object",
            "additionalProperties": False,
            "required": [
                "external_supplement_used",
                "external_network_used",
                "unknowns_must_remain_unknown",
            ],
            "properties": {
                "external_supplement_used": {"type": "boolean", "const": False},
                "external_network_used": {"type": "boolean", "const": False},
                "unknowns_must_remain_unknown": {
                    "type": "boolean",
                    "const": True,
                },
            },
        },
        "si_dependencies": {
            "type": "array",
            "minItems": 4,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "dependency_id",
                    "main_text_anchor",
                    "confirmed_scope",
                    "recoverability",
                ],
                "properties": {
                    "dependency_id": NONEMPTY,
                    "main_text_anchor": NONEMPTY,
                    "confirmed_scope": NONEMPTY,
                    "recoverability": {
                        "type": "string",
                        "enum": [
                            "derivable",
                            "partially_derivable",
                            "partially_identifiable_but_insufficient_for_replication",
                            "not_identifiable",
                        ],
                    },
                },
            },
        },
    },
}


THEORY_INFERENCE_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "benchmark_id",
        "external_supplement_used",
        "sections",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "benchmark_id": {
            "type": "string",
            "const": "test2-blind-supplement-reproduction",
        },
        "external_supplement_used": {"type": "boolean", "const": False},
        "sections": {
            "type": "array",
            "minItems": 2,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "section_id",
                    "status",
                    "evidence",
                    "reconstruction",
                    "validation_ids",
                    "residual_unknowns",
                ],
                "properties": {
                    "section_id": NONEMPTY,
                    "status": {
                        "type": "string",
                        "enum": [
                            "derived",
                            "operator_identified",
                            "not_identifiable",
                        ],
                    },
                    "evidence": STRING_ARRAY,
                    "reconstruction": NONEMPTY,
                    "validation_ids": STRING_ARRAY,
                    "residual_unknowns": STRING_ARRAY,
                },
            },
        },
    },
}


METHODS_INFERENCE_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "benchmark_id",
        "confirmed_facts",
        "figure_s1",
        "missing_fields",
        "replication_status",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "benchmark_id": {
            "type": "string",
            "const": "test2-blind-supplement-reproduction",
        },
        "confirmed_facts": {
            "type": "array",
            "minItems": 8,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["fact_id", "value", "anchor"],
                "properties": {
                    "fact_id": NONEMPTY,
                    "value": NONEMPTY,
                    "anchor": NONEMPTY,
                },
            },
        },
        "figure_s1": {
            "type": "object",
            "additionalProperties": False,
            "required": [
                "comparison",
                "parameters",
                "reported_outcome",
                "quantitative_reproduction",
            ],
            "properties": {
                "comparison": NONEMPTY,
                "parameters": STRING_ARRAY,
                "reported_outcome": NONEMPTY,
                "quantitative_reproduction": {
                    "type": "string",
                    "const": "blocked",
                },
            },
        },
        "missing_fields": {
            "type": "array",
            "minItems": 6,
            "items": NONEMPTY,
        },
        "replication_status": {
            "type": "string",
            "const": "blocked_without_raw_methods_and_data",
        },
    },
}


VALIDATION_REPORT_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "benchmark_id",
        "paper",
        "policy",
        "summary",
        "checks",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "benchmark_id": {
            "type": "string",
            "const": "test2-blind-supplement-reproduction",
        },
        "paper": {"type": "object"},
        "policy": {
            "type": "object",
            "properties": {
                "external_supplement_used": {
                    "type": "boolean",
                    "const": False,
                }
            },
            "required": ["external_supplement_used"],
        },
        "summary": {
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": [
                        "FAILED",
                        "PARTIAL_REPRODUCTION",
                        "FULL_REPRODUCTION",
                    ],
                },
                "passed": {"type": "boolean"},
                "fully_reproduced": {"type": "boolean"},
                "counts": {
                    "type": "object",
                    "properties": {
                        "PASS": {"type": "integer", "minimum": 0},
                        "FAIL": {"type": "integer", "minimum": 0},
                        "BLOCKED": {"type": "integer", "minimum": 0},
                    },
                    "required": ["PASS", "FAIL", "BLOCKED"],
                },
            },
            "required": [
                "status",
                "passed",
                "fully_reproduced",
                "counts",
            ],
        },
        "checks": {
            "type": "array",
            "minItems": 10,
            "items": {
                "type": "object",
                "properties": {
                    "check_id": NONEMPTY,
                    "title": NONEMPTY,
                    "status": {
                        "type": "string",
                        "enum": ["PASS", "FAIL", "BLOCKED"],
                    },
                    "evidence": STRING_ARRAY,
                    "interpretation": NONEMPTY,
                },
                "required": [
                    "check_id",
                    "title",
                    "status",
                    "evidence",
                    "observed",
                    "expected",
                    "tolerance",
                    "interpretation",
                ],
            },
        },
    },
}


ASSESSMENT_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "benchmark_id",
        "reproduction_status",
        "external_supplement_used",
        "historical_identity_verified",
        "validated_claims",
        "blocked_claims",
        "conclusion",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "benchmark_id": {
            "type": "string",
            "const": "test2-blind-supplement-reproduction",
        },
        "reproduction_status": {
            "type": "string",
            "const": "partial_reproduction",
        },
        "external_supplement_used": {"type": "boolean", "const": False},
        "historical_identity_verified": {"type": "boolean", "const": False},
        "validated_claims": {
            "type": "array",
            "minItems": 8,
            "items": NONEMPTY,
        },
        "blocked_claims": {
            "type": "array",
            "minItems": 3,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["claim_id", "reason", "required_inputs"],
                "properties": {
                    "claim_id": NONEMPTY,
                    "reason": NONEMPTY,
                    "required_inputs": STRING_ARRAY,
                },
            },
        },
        "conclusion": {"type": "string", "minLength": 80},
    },
}


ARTIFACT_MANIFEST_SCHEMA: dict[str, object] = {
    "type": "object",
    "additionalProperties": False,
    "required": [
        "schema_version",
        "benchmark_id",
        "entries",
    ],
    "properties": {
        "schema_version": {"type": "string", "const": "1.0"},
        "benchmark_id": {
            "type": "string",
            "const": "test2-blind-supplement-reproduction",
        },
        "entries": {
            "type": "array",
            "minItems": 6,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "role",
                    "path",
                    "digest",
                    "verification",
                ],
                "properties": {
                    "role": NONEMPTY,
                    "path": NONEMPTY,
                    "digest": DIGEST,
                    "verification": {
                        "type": "string",
                        "const": "clef_verified",
                    },
                },
            },
        },
    },
}


__all__ = [
    "ARTIFACT_MANIFEST_SCHEMA",
    "ASSESSMENT_SCHEMA",
    "EVIDENCE_LEDGER_SCHEMA",
    "METHODS_INFERENCE_SCHEMA",
    "THEORY_INFERENCE_SCHEMA",
    "VALIDATION_REPORT_SCHEMA",
]
