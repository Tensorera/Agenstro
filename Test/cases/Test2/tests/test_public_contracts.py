# ruff: noqa: D100, D101, D102, I001

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any


TEST2_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = TEST2_ROOT.parents[2]
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from reproduction.analysis import build_validation_report  # noqa: E402
from reproduction.schemas import VALIDATION_REPORT_SCHEMA  # noqa: E402
from reproduction.workflow import build_reproduction_plan  # noqa: E402


EXPECTED_EVIDENCE_DEPENDENCIES = {
    "SI-THEORY-OBLIQUE-BASIS": {
        "recoverability": "partially_derivable",
        "required_anchor_tokens": {"Eq. (48)"},
        "required_scope_tokens": {"T_I", "T_IV", "homogeneous", "forced"},
    },
    "SI-THEORY-POLYNOMIAL": {
        "recoverability": "derivable",
        "required_anchor_tokens": {"Section 3.2", "Eq. (66)"},
        "required_scope_tokens": {
            "polynomial",
            "displacement",
            "twist",
            "curvature",
        },
    },
    "SI-METHODS-EXPERIMENT-FEA": {
        "recoverability": (
            "partially_identifiable_but_insufficient_for_replication"
        ),
        "required_anchor_tokens": {"Section 3.3"},
        "required_scope_tokens": {"experiment", "ABAQUS"},
    },
    "SI-FIGURE-S1": {
        "recoverability": "partially_derivable",
        "required_anchor_tokens": {"Section 3.3", "Fig. S1"},
        "required_scope_tokens": {"substrate", "b=0.15", "40%"},
    },
}

EXPECTED_METHOD_FACTS = {
    "EXP-MATERIAL": {"PET", "40 um", "3.5 GPa", "nu=0.39", "Section 3.3"},
    "EXP-GEOMETRY": {"1:30:900", "Section 3.3"},
    "EXP-END-COMPRESSION": {"30%", "three-shape", "Fig. 3 caption"},
    "EXP-SHAPE-B": {
        "0.15",
        "0.25",
        "2*pi/3",
        "sinusoidal",
        "polynomial",
        "arc",
        "Fig. 3 caption",
    },
    "EXP-OBLIQUE": {
        "sinusoidal",
        "b=0.1",
        "30%",
        "10 and 30 deg",
        "Fig. 4 caption",
    },
    "FEA-SOFTWARE": {"ABAQUS", "version not stated", "Section 3.3"},
    "FEA-ELEMENTS": {"C3D8R", "S4R", "refined mesh", "Section 3.3"},
    "FIG-S1-LOAD": {
        "sinusoidal",
        "b=0.15",
        "40%",
        "66.7%",
        "Fig. S1",
    },
}

EXPECTED_FIGURE_S1_TOKENS = {
    "with elastomer substrate",
    "without substrate",
    "b=0.15",
    "40%",
    "66.7%",
    "c3d8r",
    "s4r",
    "almost unchanged",
}

EXPECTED_MISSING_FIELD_ALTERNATIVES = (
    ("abaqus version",),
    ("mesh size", "element count"),
    ("constitutive",),
    ("contact", "bond", "tie"),
    ("solver", "analysis procedure"),
    ("sample size", "repeat"),
    ("calibration", "coordinate extraction"),
    ("raw coordinates", "uncertainty", "error bar"),
)

EXPECTED_METHOD_REPORT_TOKENS = {
    "c3d8r",
    "s4r",
    "blocked",
    "样本量",
    "网格",
}

EXPECTED_METHOD_FORBIDDEN_CLAIMS = {
    "quantitatively reproduced fig. s1",
    "fig. s1 quantitatively reproduced",
    "fully reproducible abaqus model",
    "实验统计已完整复现",
    "fig. s1 全场已复现",
}

EXPECTED_FINAL_SECTIONS = {
    "证据账本",
    "多项式形状",
    "斜压缩",
    "实验与 FEA",
    "数值验证",
    "BLOCKED",
}

EXPECTED_BLOCKER_TOKENS = {
    "LIMIT-OBLIQUE-001": {"basis", "normalization", "publisher si"},
    "LIMIT-FEA-001": {"abaqus", "mesh", "field"},
    "LIMIT-EXPERIMENT-001": {"raw", "sample", "uncertainty"},
}

EXPECTED_FINAL_CROSS_CHECK_TOKENS = {
    "phi_(1)b(1)",
    "u_2(2)b(1)",
    "kappa_3",
    "limit-oblique-001",
    "limit-fea-001",
    "limit-experiment-001",
}

EXPECTED_FINAL_FORBIDDEN_CLAIMS = {
    "quantitatively reproduced fig. s1",
    "fig. s1 quantitatively reproduced",
    "historical t_i...t_iv fully recovered",
    "full reproduction",
    "实验统计已完整复现",
    "fig. s1 全场已复现",
}

EXPECTED_MANIFEST_SLOTS = {
    "evidence_ledger": "Evidence/evidence-ledger.json",
    "theory_inference": "Inference/Theory/theory-inference.json",
    "methods_inference": "Inference/Methods/methods-inference.json",
    "validation_report": "Validation/validation-report.json",
    "inferred_supplement": "Report/inferred-supplement.md",
    "assessment": "Report/reproduction-assessment.json",
}

EXPECTED_ADDITIONAL_VALIDATED_CLAIMS = {
    "SI-THEORY-POLYNOMIAL",
    *EXPECTED_METHOD_FACTS,
}

EXPECTED_NUMERIC_EVIDENCE_ANCHORS = {
    "SRC-001": {"Testarticle.pdf SHA-256"},
    "SRC-002": {"review-work/Extractedmd/full.md SHA-256"},
    "SI-SCOPE-001": {
        "Eq. (48)",
        "Section 3.2",
        "Section 3.3",
        "PDF page 231",
    },
    "NUM-BMAX-001": {"(22)", "(66)", "Section 3.2"},
    "NUM-FIG5-001": {"(68)", "Fig. 5", "Section 3.4"},
    "NUM-FIG8-001": {
        "(34)",
        "(52)",
        "(55)",
        "(66)",
        "(69)",
    },
    "NUM-PRESTRAIN-001": {"Section 3.3", "Section 3.4", "Fig. S1"},
    "NUM-FIG10-001": {"(73)", "Fig. 10(c)"},
    "NUM-STRAIGHT-001": {
        "(52)",
        "(53)",
        "(60)",
        "(61)",
        "Fig. 8",
    },
    "LIMIT-OBLIQUE-001": {"Eq. (47)", "Eq. (48)"},
    "LIMIT-FEA-001": {"Section 3.3", "Fig. S1"},
    "LIMIT-EXPERIMENT-001": {"Section 3.3", "Fig. 3", "Fig. 4"},
}

EXPECTED_NUMERIC_REQUIRED_PATHS = {
    "SRC-001": {"$"},
    "SRC-002": {"$"},
    "SI-SCOPE-001": {
        "oblique_basis",
        "polynomial_results",
        "experiment_fea_details",
        "figure_s1",
    },
    "NUM-BMAX-001": {"sinusoidal", "polynomial", "arc"},
    "NUM-FIG5-001": {"initial_shape", "initial_curvature"},
    "NUM-FIG8-001": {
        "mode_ratio.sinusoidal",
        "mode_ratio.polynomial",
        "mode_ratio.arc",
        "polynomial_completion.sample_x",
        "polynomial_completion.kappa3",
        "polynomial_completion.phi",
        "polynomial_completion.u2",
    },
    "NUM-PRESTRAIN-001": {"66.67_percent", "70_percent"},
    "NUM-FIG10-001": {"0.1", "0.3", "0.5"},
    "NUM-STRAIGHT-001": {
        "critical_load_coefficient",
        "u_app_second_order_coefficient",
        "normalized_maximum_strain",
    },
    "LIMIT-OBLIQUE-001": {"$"},
    "LIMIT-FEA-001": {"$"},
    "LIMIT-EXPERIMENT-001": {"$"},
}


def _all_scalars(value: Any) -> list[Any]:
    if isinstance(value, dict):
        return [
            scalar
            for item in value.values()
            for scalar in _all_scalars(item)
        ]
    if isinstance(value, list):
        return [scalar for item in value for scalar in _all_scalars(item)]
    return [value]


def _casefolded_strings(value: Any) -> set[str]:
    return {
        scalar.casefold()
        for scalar in _all_scalars(value)
        if isinstance(scalar, str)
    }


class PublicAcceptanceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.benchmark = json.loads(
            (TEST2_ROOT / "reproduction" / "benchmark-spec.json").read_text(
                encoding="utf-8"
            )
        )
        cls.baseline_numeric_report = build_validation_report(
            TEST2_ROOT / "Testarticle.pdf",
            TEST2_ROOT / "review-work" / "Extractedmd" / "full.md",
        )
        cls.verifier_source = (
            TEST2_ROOT / "reproduction" / "verification.py"
        ).read_text(encoding="utf-8")

    def _task_prompts(self) -> dict[str, str]:
        with tempfile.TemporaryDirectory(
            prefix="repro-public-contract-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder = Path(temporary) / "work"
            workfolder.mkdir()
            plan = build_reproduction_plan(TEST2_ROOT, workfolder)
            return {
                task_id: "\n".join(prompt.content for prompt in task.prompts)
                for task_id, task in plan.tasks.items()
            }

    def _assert_verifier_reads_contract(self, contract_name: str) -> None:
        direct_read = f'benchmark.get("{contract_name}")'
        helper_read = f'_public_contract(context, "{contract_name}")'
        self.assertTrue(
            direct_read in self.verifier_source
            or helper_read in self.verifier_source,
            f"verifier does not read {contract_name}",
        )

    def test_theory_contract_is_public_and_bound_end_to_end(self) -> None:
        contract = self.benchmark["theory_output_contract"]
        sections = {
            section["section_id"]: section for section in contract["sections"]
        }
        self.assertEqual(
            set(sections),
            {"SI-THEORY-POLYNOMIAL", "SI-THEORY-OBLIQUE-BASIS"},
        )
        for section in sections.values():
            self.assertIn("status", section)
            self.assertIn("validation_ids", section)
            self.assertIn("required_evidence_anchors", section)
            self.assertIn("required_reconstruction_tokens", section)
            self.assertIn("required_contract_tokens", section)

        prompts = self._task_prompts()
        self.assertIn(
            "benchmark_spec.theory_output_contract",
            prompts["infer-theory-supplement"],
        )
        self._assert_verifier_reads_contract("theory_output_contract")

    def test_evidence_contract_exposes_the_fixed_dependency_inventory(
        self,
    ) -> None:
        contract = self.benchmark["evidence_output_contract"]
        dependencies = {
            item["dependency_id"]: item for item in contract["dependencies"]
        }
        self.assertEqual(set(dependencies), set(EXPECTED_EVIDENCE_DEPENDENCIES))
        for dependency_id, expected in EXPECTED_EVIDENCE_DEPENDENCIES.items():
            actual = dependencies[dependency_id]
            self.assertEqual(
                actual["recoverability"],
                expected["recoverability"],
            )
            for field in (
                "required_anchor_tokens",
                "required_scope_tokens",
            ):
                self.assertEqual(set(actual[field]), expected[field])

        prompts = self._task_prompts()
        self.assertIn(
            "benchmark_spec.evidence_output_contract",
            prompts["inventory-supplement-evidence"],
        )
        self._assert_verifier_reads_contract("evidence_output_contract")

    def test_numeric_contract_exposes_all_checks_without_order_coupling(
        self,
    ) -> None:
        contract = self.benchmark.get("numeric_output_contract")
        self.assertIsInstance(
            contract,
            dict,
            "benchmark must publish numeric_output_contract",
        )
        self.assertIs(
            contract.get("check_order_significant"),
            False,
            "numeric verifier must accept checks by ID, not list order",
        )
        self.assertEqual(
            contract.get("summary_status"),
            "PARTIAL_REPRODUCTION",
        )
        self.assertEqual(
            set(contract.get("presentation_fields_ignored", [])),
            {"title", "interpretation"},
        )
        public_checks = contract.get("checks")
        self.assertIsInstance(public_checks, list)
        by_id = {
            item.get("check_id"): item
            for item in public_checks
            if isinstance(item, dict)
            and isinstance(item.get("check_id"), str)
        }
        expected_checks = {
            item["check_id"]: item
            for item in self.baseline_numeric_report["checks"]
        }
        self.assertEqual(len(public_checks), 12)
        self.assertEqual(set(by_id), set(expected_checks))
        for check_id, expected in expected_checks.items():
            public = by_id[check_id]
            self.assertEqual(
                public.get("status"),
                expected["status"],
                f"{check_id} hides its required status",
            )
            for field in (
                "required_observed_paths",
                "required_expected_paths",
            ):
                required_paths = public.get(field)
                self.assertIsInstance(
                    required_paths,
                    list,
                    f"{check_id} must publish {field}",
                )
                self.assertTrue(
                    required_paths,
                    f"{check_id} has no {field}",
                )
                self.assertTrue(
                    all(
                        isinstance(path, str) and path
                        for path in required_paths
                    ),
                    f"{check_id} has an invalid path in {field}",
                )
                self.assertEqual(
                    set(required_paths),
                    EXPECTED_NUMERIC_REQUIRED_PATHS[check_id],
                    f"{check_id} hides a required path in {field}",
                )
            self.assertIn(
                "comparison",
                public,
                f"{check_id} hides its comparison contract",
            )
            self.assertEqual(
                set(public.get("required_evidence_anchors", [])),
                EXPECTED_NUMERIC_EVIDENCE_ANCHORS[check_id],
                f"{check_id} hides evidence anchors",
            )

    def test_numeric_prompt_and_verifier_read_the_public_contract(self) -> None:
        prompts = self._task_prompts()
        self.assertIn(
            "benchmark_spec.numeric_output_contract",
            prompts["validate-paper-numerics"],
        )
        self._assert_verifier_reads_contract("numeric_output_contract")

    def test_methods_contract_exposes_every_fixed_verifier_requirement(
        self,
    ) -> None:
        contract = self.benchmark["methods_output_contract"]
        facts = {
            item["fact_id"]: item for item in contract["confirmed_facts"]
        }
        self.assertEqual(set(facts), set(EXPECTED_METHOD_FACTS))
        for fact_id, tokens in EXPECTED_METHOD_FACTS.items():
            actual = _casefolded_strings(facts[fact_id])
            self.assertFalse(
                {
                    token
                    for token in tokens
                    if token.casefold() not in actual
                },
                f"{fact_id} hides a required semantic token",
            )

        figure_tokens = _casefolded_strings(contract["figure_s1"])
        self.assertFalse(
            {
                token
                for token in EXPECTED_FIGURE_S1_TOKENS
                if token.casefold() not in figure_tokens
            }
        )
        self.assertEqual(
            contract["figure_s1"]["quantitative_reproduction"],
            "blocked",
        )

        public_categories = [
            {token.casefold() for token in category}
            for category in contract["missing_field_categories"]
        ]
        for alternatives in EXPECTED_MISSING_FIELD_ALTERNATIVES:
            folded = {token.casefold() for token in alternatives}
            self.assertTrue(
                any(folded.issubset(category) for category in public_categories),
                f"methods contract hides category {alternatives}",
            )

        self.assertEqual(
            contract["replication_status"],
            "blocked_without_raw_methods_and_data",
        )
        contract_strings = _casefolded_strings(contract)
        for token in EXPECTED_METHOD_REPORT_TOKENS:
            self.assertIn(token.casefold(), contract_strings)
        self.assertEqual(
            {
                token.casefold()
                for token in contract["forbidden_claims"]
            },
            {
                token.casefold()
                for token in EXPECTED_METHOD_FORBIDDEN_CLAIMS
            },
        )

    def test_methods_prompt_and_verifier_read_the_public_contract(self) -> None:
        prompts = self._task_prompts()
        self.assertIn(
            "benchmark_spec.methods_output_contract",
            prompts["infer-methods-supplement"],
        )
        self._assert_verifier_reads_contract("methods_output_contract")
        self.assertNotIn("_EXPECTED_METHOD_FACTS", self.verifier_source)

    def test_final_contract_exposes_every_fixed_verifier_requirement(
        self,
    ) -> None:
        contract = self.benchmark["final_output_contract"]
        section_groups = [
            {token.casefold() for token in alternatives}
            for alternatives in contract["required_section_token_alternatives"]
        ]
        for section in EXPECTED_FINAL_SECTIONS:
            self.assertTrue(
                any(section.casefold() in group for group in section_groups),
                f"final contract hides required section {section}",
            )
        self.assertIn(
            "publisher supplementary file was not used",
            {
                token.casefold()
                for token in contract["blind_source_declaration_alternatives"]
            },
        )
        self.assertEqual(contract["reproduction_status"], "partial_reproduction")
        self.assertIs(contract["external_supplement_used"], False)

        blocked = {
            item["claim_id"]: _casefolded_strings(item)
            for item in contract["blocked_claims"]
        }
        self.assertEqual(set(blocked), set(EXPECTED_BLOCKER_TOKENS))
        for claim_id, tokens in EXPECTED_BLOCKER_TOKENS.items():
            for token in tokens:
                self.assertIn(token.casefold(), blocked[claim_id])

        self.assertEqual(
            {
                token.casefold()
                for token in contract["required_supplement_tokens"]
            },
            {
                token.casefold()
                for token in EXPECTED_FINAL_CROSS_CHECK_TOKENS
            },
        )
        self.assertEqual(
            {
                token.casefold()
                for token in contract["forbidden_claims"]
            },
            {
                token.casefold()
                for token in EXPECTED_FINAL_FORBIDDEN_CLAIMS
            },
        )
        manifest = {
            item["role"]: item["path"] for item in contract["manifest_slots"]
        }
        self.assertEqual(manifest, EXPECTED_MANIFEST_SLOTS)
        self.assertEqual(
            contract["validation_report_policy"],
            {
                "summary_counts_are_derived_from_checks": True,
                "maximum_fail_count": 0,
            },
        )
        claim_policy = contract["validated_claims_policy"]
        self.assertEqual(
            claim_policy["entry_format"],
            {
                "plain_claim_id_allowed": True,
                "described_claim_separator": ":",
                "described_claim_requires_nonempty_description": True,
            },
        )
        self.assertEqual(claim_policy["required_numeric_status"], "PASS")
        self.assertEqual(claim_policy["numeric_coverage"], "all")
        self.assertEqual(claim_policy["blocked_numeric_status"], "BLOCKED")
        self.assertEqual(
            claim_policy["blocked_claim_promotion"],
            "forbidden",
        )
        additional = {
            item["claim_id"]: item
            for item in claim_policy["permitted_additional_claims"]
        }
        self.assertEqual(
            set(additional),
            EXPECTED_ADDITIONAL_VALIDATED_CLAIMS,
        )
        theory_claim = additional["SI-THEORY-POLYNOMIAL"]
        self.assertEqual(
            theory_claim["evidence_input"],
            "infer_theory_supplement_theory_inference",
        )
        self.assertEqual(theory_claim["required_field_values"], {"status": "derived"})
        self.assertEqual(
            set(theory_claim["required_list_members"]["validation_ids"]),
            {"NUM-FIG8-001", "NUM-BMAX-001"},
        )
        for fact_id in EXPECTED_METHOD_FACTS:
            self.assertEqual(
                additional[fact_id]["evidence_input"],
                "infer_methods_supplement_methods_inference",
            )
            self.assertEqual(
                additional[fact_id]["evidence_collection"],
                "confirmed_facts",
            )
            self.assertEqual(
                set(additional[fact_id]["required_nonempty_fields"]),
                {"value", "anchor"},
            )
        self.assertEqual(
            contract["blocked_claims_policy"],
            {
                "required_numeric_status": "BLOCKED",
                "numeric_coverage": "exact",
                "public_contract_coverage": "exact",
                "duplicate_claim_ids": "forbidden",
            },
        )
        self.assertEqual(
            contract["manifest_policy"],
            {
                "entry_order_significant": False,
                "roles": "exactly_manifest_slots",
                "paths": "exactly_slot_path",
                "digest": "sha256_of_bound_input_or_self_output",
                "additional_entries": "forbidden",
            },
        )

    def test_final_prompt_and_verifier_read_the_public_contract(self) -> None:
        prompts = self._task_prompts()
        self.assertIn(
            "benchmark_spec.final_output_contract",
            prompts["synthesize-inferred-supplement"],
        )
        self.assertIn(
            "validated_claims_policy",
            prompts["synthesize-inferred-supplement"],
        )
        self._assert_verifier_reads_contract("final_output_contract")
        self.assertNotIn("_MANIFEST_SLOTS", self.verifier_source)


class HonestFailureSchemaTests(unittest.TestCase):
    def test_numeric_report_schema_can_represent_an_honest_failure(self) -> None:
        properties = VALIDATION_REPORT_SCHEMA["properties"]
        summary = properties["summary"]["properties"]
        check = properties["checks"]["items"]["properties"]

        self.assertIn("FAILED", summary["status"]["enum"])
        self.assertNotIn("const", summary["passed"])
        self.assertNotIn("const", summary["counts"]["properties"]["FAIL"])
        self.assertIn("FAIL", check["status"]["enum"])


if __name__ == "__main__":
    unittest.main()
