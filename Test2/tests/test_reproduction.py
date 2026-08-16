from __future__ import annotations

import json
import math
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

TEST2_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = TEST2_ROOT.parent
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from reproduction.analysis import (  # noqa: E402
    b_max_values,
    build_validation_report,
    closed_form_mode_ratio_coefficient,
    compression_from_prestrain,
    first_order_mode_ratio_coefficient,
    inverse_design_strain,
    polynomial_completion_diagnostics,
    sinusoidal_initial_errors,
)
from reproduction.offline import make_offline_callback  # noqa: E402
from reproduction.run import (  # noqa: E402
    _agent_adapter,
    bind_reproduction_profile,
)
from reproduction.verification import (  # noqa: E402
    build_reproduction_registry,
)
from reproduction.workflow import build_reproduction_plan  # noqa: E402

from clef_sdk.adapters import FakeAdapter  # noqa: E402
from clef_sdk.compiler import compile_plan  # noqa: E402
from clef_sdk.model import ArtifactRef, WorkflowState  # noqa: E402
from clef_sdk.profiles import ModelRoute  # noqa: E402
from clef_sdk.runtime import execute_plan  # noqa: E402

PDF = TEST2_ROOT / "Testarticle.pdf"
MARKDOWN = TEST2_ROOT / "review-work" / "Extractedmd" / "full.md"
PROFILE = TEST2_ROOT / "reproduction" / "reproduction_profile.toml"


class ReproductionNumericsTests(unittest.TestCase):
    def test_geometric_b_limits_match_paper(self) -> None:
        values = b_max_values()
        self.assertAlmostEqual(values["sinusoidal"], 0.159, delta=5e-4)
        self.assertAlmostEqual(values["polynomial"], 0.291, delta=5e-4)
        self.assertAlmostEqual(values["arc"], math.pi, places=12)

    def test_figure5_third_order_errors_are_reproduced(self) -> None:
        values = sinusoidal_initial_errors(0.1, 3)
        self.assertAlmostEqual(values["initial_shape"], 0.00906, delta=5e-5)
        self.assertAlmostEqual(values["initial_curvature"], 0.0173, delta=5e-5)

    def test_missing_polynomial_mode_result_is_reconstructed(self) -> None:
        for shape in ("sinusoidal", "polynomial", "arc"):
            numerical = first_order_mode_ratio_coefficient(shape, 0.39)
            closed_form = closed_form_mode_ratio_coefficient(shape, 0.39)
            self.assertAlmostEqual(numerical, closed_form, delta=2e-7)

    def test_all_three_polynomial_si_quantities_are_reconstructed(self) -> None:
        diagnostics = polynomial_completion_diagnostics()
        self.assertAlmostEqual(
            diagnostics["kappa3"][1], -3.0399993234497518, delta=5e-6
        )
        self.assertAlmostEqual(diagnostics["phi"][2], -1.8886713243831512, delta=1e-6)
        self.assertAlmostEqual(diagnostics["u2"][1], -2.5847251916185936, delta=2e-6)
        self.assertLess(diagnostics["phi_endpoint_residual"], 1e-10)
        self.assertLess(diagnostics["phi_derivative_residual"], 2e-5)
        self.assertLess(diagnostics["u2_endpoint_residual"], 1e-10)
        self.assertLess(diagnostics["u2_equation_residual"], 3e-4)
        self.assertLess(diagnostics["kappa3_odd_residual"], 1e-10)
        self.assertLess(diagnostics["phi_even_residual"], 1e-10)
        self.assertLess(diagnostics["u2_even_residual"], 1e-10)
        self.assertLess(diagnostics["arc_closed_form_residual"], 5e-6)

    def test_prestrain_conversion_matches_figure_s1_statement(self) -> None:
        self.assertAlmostEqual(compression_from_prestrain(2.0 / 3.0), 0.4, places=12)
        self.assertAlmostEqual(compression_from_prestrain(0.7), 0.412, delta=5e-4)

    def test_inverse_design_matches_figure10_examples(self) -> None:
        expected = {0.1: 0.0235, 0.3: 0.157, 0.5: 0.297}
        for ratio, figure_value in expected.items():
            self.assertAlmostEqual(
                inverse_design_strain(ratio),
                figure_value,
                delta=0.005,
            )

    def test_full_baseline_has_no_failures_and_expected_blockers(self) -> None:
        report = build_validation_report(PDF, MARKDOWN)
        self.assertTrue(report["summary"]["passed"])
        self.assertEqual(report["summary"]["counts"]["PASS"], 9)
        self.assertEqual(report["summary"]["counts"]["FAIL"], 0)
        self.assertEqual(report["summary"]["counts"]["BLOCKED"], 3)


class ReproductionClefTests(unittest.TestCase):
    def _fixture(self, temporary: str):
        workfolder = Path(temporary) / "work"
        workfolder.mkdir()
        profile = bind_reproduction_profile(PROFILE, workfolder)
        plan = build_reproduction_plan(TEST2_ROOT, workfolder)
        compiled = compile_plan(plan, profile)
        return workfolder, profile, compiled

    def test_plan_is_one_to_three_to_one_dag(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-plan-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            _workfolder, _profile, compiled = self._fixture(temporary)
            self.assertEqual(len(compiled.plan.tasks), 5)
            self.assertEqual(len(compiled.plan.bindings), 11)
            final = compiled.plan.tasks["synthesize-inferred-supplement"]
            self.assertEqual(final.metadata["topo_rank"], 2)

    def test_live_inputs_and_environment_are_blind_allowlists(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-blind-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder, profile, compiled = self._fixture(temporary)
            bundle = (workfolder / "0000_blind_inputs").resolve(strict=True)
            self.assertEqual(profile.workspace.read_roots, (bundle,))
            self.assertEqual(
                {
                    path.relative_to(bundle).as_posix()
                    for path in bundle.rglob("*")
                    if path.is_file()
                },
                {
                    "benchmark-spec.json",
                    "input-bundle.json",
                    "manuscript.md",
                    "manuscript.pdf",
                    "prior-decision.json",
                    "prior-review.md",
                },
            )
            for task in compiled.plan.tasks.values():
                for value in task.inputs.values():
                    if isinstance(value, ArtifactRef):
                        path = Path(value.uri).resolve(strict=True)
                        self.assertTrue(path.is_relative_to(bundle), path)
            public_benchmark = json.loads(
                (bundle / "benchmark-spec.json").read_text(encoding="utf-8")
            )
            theory_contract = public_benchmark["theory_output_contract"]
            self.assertEqual(
                {
                    section["section_id"]
                    for section in theory_contract["sections"]
                },
                {
                    "SI-THEORY-POLYNOMIAL",
                    "SI-THEORY-OBLIQUE-BASIS",
                },
            )
            theory_task = compiled.plan.tasks["infer-theory-supplement"]
            self.assertIn(
                "benchmark_spec.theory_output_contract",
                "\n".join(prompt.content for prompt in theory_task.prompts),
            )
            with patch.dict(
                "os.environ",
                {
                    "PATH": "safe-path",
                    "SystemRoot": "safe-root",
                    "OPENAI_API_KEY": "must-not-pass",
                    "GH_TOKEN": "must-not-pass",
                    "MINERU_API": "must-not-pass",
                },
                clear=True,
            ):
                profile = replace(
                    profile,
                    adapter=replace(
                        profile.adapter,
                        models={"low": ModelRoute("provider/model", "xhigh")},
                    ),
                )
                adapter = _agent_adapter(profile)
            self.assertFalse(adapter.inherit_environment)
            environment = adapter.environment or {}
            self.assertEqual(
                {
                    key.casefold(): value
                    for key, value in environment.items()
                    if key.casefold() != "opencode_config_content"
                },
                {"path": "safe-path", "systemroot": "safe-root"},
            )
            security = json.loads(environment["OPENCODE_CONFIG_CONTENT"])
            self.assertEqual(security["share"], "disabled")
            self.assertEqual(security["permission"]["external_directory"]["*"], "deny")
            self.assertEqual(security["permission"]["webfetch"], "deny")
            serialized_security = json.dumps(security)
            self.assertNotIn("output-final", serialized_security)
            external_rules = security["permission"]["external_directory"]
            test2_posix = TEST2_ROOT.resolve().as_posix()
            self.assertNotIn(test2_posix, external_rules)
            self.assertNotIn(f"{test2_posix}/**", external_rules)
            self.assertEqual(adapter.models, profile.adapter.models)

    def test_fake_adapter_executes_and_publishes_verified_bundle(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-e2e-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder, profile, compiled = self._fixture(temporary)
            callback = make_offline_callback(PDF, MARKDOWN)
            result = execute_plan(
                compiled.plan,
                profile=profile,
                adapter=FakeAdapter([callback] * 5),
                verifier_registry=build_reproduction_registry(),
            )
            self.assertIs(
                result.state,
                WorkflowState.SUCCEEDED,
                json.dumps(result.to_dict(), ensure_ascii=False, indent=2),
            )
            self.assertEqual(len(result.task_results), 5)
            self.assertTrue(
                (workfolder / "Report" / "inferred-supplement.md").is_file()
            )
            assessment = json.loads(
                (workfolder / "Report" / "reproduction-assessment.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(
                assessment["reproduction_status"],
                "partial_reproduction",
            )
            self.assertFalse(assessment["external_supplement_used"])

    def test_tampered_numeric_artifact_blocks_final_synthesis(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-tamper-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder, profile, compiled = self._fixture(temporary)
            callback = make_offline_callback(
                PDF,
                MARKDOWN,
                tamper_numeric=True,
            )
            result = execute_plan(
                compiled.plan,
                profile=profile,
                adapter=FakeAdapter([callback] * 20),
                verifier_registry=build_reproduction_registry(),
            )
            self.assertIs(result.state, WorkflowState.FAILED)
            self.assertIn(
                "synthesize-inferred-supplement",
                result.skipped_tasks,
            )
            self.assertFalse(
                (workfolder / "Report" / "inferred-supplement.md").exists()
            )

    def _assert_upstream_tamper_blocks_final(self, **tamper) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-semantic-tamper-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder, profile, compiled = self._fixture(temporary)
            callback = make_offline_callback(PDF, MARKDOWN, **tamper)
            result = execute_plan(
                compiled.plan,
                profile=profile,
                adapter=FakeAdapter([callback] * 20),
                verifier_registry=build_reproduction_registry(),
            )
            self.assertIs(result.state, WorkflowState.FAILED)
            self.assertIn(
                "synthesize-inferred-supplement",
                result.skipped_tasks,
            )
            self.assertFalse(
                (workfolder / "Report" / "inferred-supplement.md").exists()
            )

    def test_tampered_theory_blocks_final_synthesis(self) -> None:
        self._assert_upstream_tamper_blocks_final(tamper_theory=True)

    def test_tampered_methods_blocks_final_synthesis(self) -> None:
        self._assert_upstream_tamper_blocks_final(tamper_methods=True)

    def test_tampered_final_claim_is_not_published(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-final-tamper-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder, profile, compiled = self._fixture(temporary)
            callback = make_offline_callback(PDF, MARKDOWN, tamper_final=True)
            result = execute_plan(
                compiled.plan,
                profile=profile,
                adapter=FakeAdapter([callback] * 20),
                verifier_registry=build_reproduction_registry(),
            )
            self.assertIs(result.state, WorkflowState.FAILED)
            self.assertNotIn(
                "synthesize-inferred-supplement",
                result.skipped_tasks,
            )
            self.assertFalse(
                (workfolder / "Report" / "inferred-supplement.md").exists()
            )

    def test_manifest_path_impersonation_is_not_published(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="repro-manifest-tamper-",
            dir=TEST2_ROOT / "reproduction",
        ) as temporary:
            workfolder, profile, compiled = self._fixture(temporary)
            callback = make_offline_callback(PDF, MARKDOWN, tamper_manifest=True)
            result = execute_plan(
                compiled.plan,
                profile=profile,
                adapter=FakeAdapter([callback] * 20),
                verifier_registry=build_reproduction_registry(),
            )
            self.assertIs(result.state, WorkflowState.FAILED)
            self.assertFalse(
                (workfolder / "Report" / "artifact-manifest.json").exists()
            )


if __name__ == "__main__":
    unittest.main()
