from __future__ import annotations

import hashlib
import json
import os
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

from clef_sdk.compiler import compile_plan  # noqa: E402
from clef_sdk.adapters import FakeAdapter  # noqa: E402
from clef_sdk.model import (  # noqa: E402
    ArtifactBinding,
    ArtifactClaim,
    EffectKind,
    RunState,
    WorkflowState,
)
from clef_sdk.protocol import (  # noqa: E402
    AgentReport,
    decode_request,
    encode_report_envelope,
)
from clef_sdk.profiles import ModelRoute  # noqa: E402
from clef_sdk.runtime import execute_plan  # noqa: E402
from script.Mineru import ExtractionResult  # noqa: E402
from script.Reviewfunction import (  # noqa: E402
    REVIEW_BUILDERS,
    REVIEW_DEFINITIONS,
    build_review_registry,
)
from script.artifacts import write_review_context  # noqa: E402
from script.runtime_profile import bind_profile  # noqa: E402
from script.orchestrator import _sanitized_agent_adapter  # noqa: E402
from script.workflow import build_review_inputs, build_review_plan  # noqa: E402


PROFILE = TEST2_ROOT / "script" / "review_profile.toml"


def _digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class ReviewWorkflowTests(unittest.TestCase):
    def _fixture(self, root: Path) -> tuple[ExtractionResult, Path]:
        root.mkdir(parents=True, exist_ok=True)
        manuscript = root / "manuscript.pdf"
        manuscript.write_bytes(b"%PDF-1.7\noffline fixture\n%%EOF\n")
        extracted = root / "Extractedmd"
        extracted.mkdir()
        full_md = extracted / "full.md"
        full_md.write_text(
            "# Offline manuscript\n\n"
            "## Abstract\nA deterministic test manuscript.\n\n"
            "## Methods\nMethod details.\n\n"
            "## Results\nResult details.\n",
            encoding="utf-8",
        )
        extraction = ExtractionResult(
            manuscript_pdf=manuscript,
            extracted_dir=extracted,
            full_markdown=full_md,
            manuscript_sha256=_digest(manuscript),
            full_markdown_sha256=_digest(full_md),
            batch_id="offline",
            reused=True,
        )
        context = write_review_context(
            root,
            manuscript_sha256=extraction.manuscript_sha256,
            full_markdown_sha256=extraction.full_markdown_sha256,
            venue=None,
            article_type="original_research",
            review_language="zh-CN",
            venue_guidelines=None,
        )
        return extraction, context

    def test_all_twelve_small_builders_are_registered(self) -> None:
        self.assertEqual(len(REVIEW_BUILDERS), 12)
        self.assertEqual(len(REVIEW_DEFINITIONS), 12)
        self.assertEqual(
            [item.review_id for item in REVIEW_DEFINITIONS],
            [f"{index:02d}" for index in range(1, 13)],
        )

    def test_plan_compiles_to_twelve_reviews_and_one_fan_in(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-plan-", dir=TEST2_ROOT
        ) as temporary:
            root = (Path(temporary) / "work").resolve()
            extraction, context = self._fixture(root)
            profile = bind_profile(PROFILE, root)
            plan = build_review_plan(extraction, context, root)
            compiled = compile_plan(plan, profile)

            self.assertEqual(len(compiled.plan.tasks), 13)
            self.assertEqual(len(compiled.plan.bindings), 24)
            self.assertEqual(
                compiled.topological_order[-1], "synthesize-final-review"
            )
            self.assertTrue(compiled.validation.passed)
            final = compiled.plan.tasks["synthesize-final-review"]
            bound = [
                value
                for value in final.inputs.values()
                if isinstance(value, ArtifactBinding)
            ]
            self.assertEqual(len(bound), 24)
            for task in compiled.plan.tasks.values():
                for output in task.outputs.values():
                    if output.path is None:
                        self.fail(f"output {output.name} has no path")
                    self.assertTrue(Path(output.path).is_relative_to(root))

    def test_prompt_material_is_loaded_into_each_task(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-prompt-", dir=TEST2_ROOT
        ) as temporary:
            root = Path(temporary).resolve()
            extraction, context = self._fixture(root)
            inputs = build_review_inputs(extraction, context)
            tasks = [builder(inputs, root) for builder in REVIEW_BUILDERS]
            self.assertEqual(len(tasks), 12)
            for task in tasks:
                joined = "\n".join(prompt.content for prompt in task.prompts)
                self.assertIn("共同审查政策", joined)
                self.assertIn("访问日期：2026-07-27", joined)
                self.assertIn("JSON Schema", joined)

    def test_all_agent_tasks_declare_shell_intent(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-shell-policy-", dir=TEST2_ROOT
        ) as temporary:
            root = (Path(temporary) / "work").resolve()
            extraction, context = self._fixture(root)
            plan = build_review_plan(extraction, context, root)

            for task in plan.tasks.values():
                declared = {rule.kind for rule in task.contract.effects.allowed}
                self.assertIn(EffectKind.SHELL, declared, task.id)

    def test_fake_adapter_executes_and_publishes_all_verified_artifacts(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-e2e-", dir=TEST2_ROOT
        ) as temporary:
            root = (Path(temporary) / "work").resolve()
            extraction, context = self._fixture(root)
            profile = bind_profile(PROFILE, root)
            plan = build_review_plan(extraction, context, root)

            def fake_agent(
                prompt: str, workspace: Path, _session_id: str | None
            ) -> str:
                marker = "Canonical AgentRequest JSON:\n"
                payload = prompt.split(marker, 1)[1].split(
                    "\n\nThe final non-whitespace", 1
                )[0]
                request = decode_request(payload)
                artifacts = []
                if request.task_id == "synthesize-final-review":
                    dimensions = [
                        {
                            "review_id": item.review_id,
                            "dimension": item.slug,
                            "verdict": "pass",
                            "rationale": "离线伪适配器确认结构化输入完整且无测试问题。",
                            "top_issue_ids": [],
                        }
                        for item in REVIEW_DEFINITIONS
                    ]
                    report = "\n".join(
                        [
                            "# 最终审稿报告",
                            "## 编辑摘要",
                            "离线验证内容。" * 1700,
                            "## 十二维度结论表",
                            *[
                                f"- {item.review_id} {item.title}: pass"
                                for item in REVIEW_DEFINITIONS
                            ],
                            "## 审查局限",
                            "这是确定性伪适配器结果，仅验证工作流。",
                        ]
                    )
                    decision = {
                        "schema_version": "1.0",
                        "recommendation": "accept",
                        "confidence": "high",
                        "executive_summary": "离线伪适配器用于验证完整工作流。" * 10,
                        "dimension_reviews": dimensions,
                        "priority_actions": [],
                        "blocking_issues": [],
                        "conflicts": [],
                        "limitations": ["仅用于离线测试。"],
                    }
                    content_by_name = {
                        "final_report": report,
                        "decision": json.dumps(
                            decision, ensure_ascii=False, indent=2
                        ),
                    }
                else:
                    item = next(
                        value
                        for value in REVIEW_DEFINITIONS
                        if value.task_id == request.task_id
                    )
                    findings = {
                        "schema_version": "1.0",
                        "review_id": item.review_id,
                        "dimension": item.slug,
                        "title": item.title,
                        "summary": "离线伪适配器完成结构和协议验证。" * 8,
                        "verdict": "pass",
                        "confidence": "high",
                        "strengths": ["测试 Artifact 结构完整。"],
                        "issues": [],
                        "questions_for_authors": [],
                        "limitations": ["未进行真实学术判断。"],
                        "reference_ids": [],
                    }
                    report = (
                        f"# {item.review_id} {item.title}\n\n"
                        "## 审查结论\n"
                        + "离线伪适配器内容用于验证 Artifact、协议和发布流程。"
                        * 100
                    )
                    content_by_name = {
                        "report": report,
                        "findings": json.dumps(
                            findings, ensure_ascii=False, indent=2
                        ),
                    }
                for output in request.expected_outputs:
                    if output.path is None:
                        self.fail(f"output {output.name} has no path")
                    path = Path(output.path)
                    self.assertTrue(path.is_relative_to(workspace))
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(
                        content_by_name[output.name], encoding="utf-8"
                    )
                    artifacts.append(
                        ArtifactClaim(
                            name=output.name,
                            uri=str(path),
                            description=output.description,
                            kind=output.kind,
                            media_type=(
                                "application/json"
                                if output.kind.value == "json"
                                else "text/markdown"
                            ),
                        )
                    )
                return encode_report_envelope(
                    AgentReport(
                        run_id=request.run_id,
                        task_id=request.task_id,
                        attempt=request.attempt,
                        text="offline fake completed",
                        state=RunState.SUCCEEDED,
                        artifacts=tuple(artifacts),
                    )
                )

            result = execute_plan(
                plan,
                profile=profile,
                adapter=FakeAdapter([fake_agent] * 13),
                verifier_registry=build_review_registry(),
            )
            self.assertIs(
                result.state,
                WorkflowState.SUCCEEDED,
                json.dumps(result.to_dict(), ensure_ascii=False, indent=2),
            )
            self.assertEqual(len(result.task_results), 13)
            self.assertTrue((root / "Report" / "final-review-report.md").is_file())
            self.assertTrue((root / "Report" / "decision.json").is_file())
            self.assertEqual(
                len(list((root / "Reviewprocess").glob("*/review.md"))), 12
            )

    def test_reference_catalog_is_valid_and_complete(self) -> None:
        catalog = json.loads(
            (
                TEST2_ROOT
                / "script"
                / "Reviewfunction"
                / "references"
                / "SOURCES.json"
            ).read_text(encoding="utf-8")
        )
        ids = {item["id"] for item in catalog["sources"]}
        self.assertEqual(len(ids), 9)
        self.assertIn("NATURE-CRITERIA", ids)
        self.assertIn("ELSEVIER-REVIEW", ids)
        self.assertIn("COPE-REVIEW", ids)

    def test_agent_environment_excludes_mineru_token(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-env-scrub-", dir=TEST2_ROOT
        ) as temporary:
            root = (Path(temporary) / "work").resolve()
            extraction, _context = self._fixture(root)
            self.assertTrue(extraction.manuscript_pdf.is_file())
            profile = bind_profile(PROFILE, root)
            profile = replace(
                profile,
                adapter=replace(
                    profile.adapter,
                    models={"high": ModelRoute("provider/model", "low")},
                ),
            )
            with patch.dict(
                os.environ,
                {"Mineru_Api": "do-not-forward", "KEEP_FOR_PROVIDER": "yes"},
                clear=True,
            ):
                adapter = _sanitized_agent_adapter(profile)
            self.assertFalse(adapter.inherit_environment)
            self.assertNotIn("Mineru_Api", adapter.environment or {})
            self.assertEqual(
                (adapter.environment or {}).get("KEEP_FOR_PROVIDER"), "yes"
            )
            self.assertEqual(adapter.models, profile.adapter.models)


if __name__ == "__main__":
    unittest.main()
