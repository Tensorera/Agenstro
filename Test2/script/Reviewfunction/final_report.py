"""Final fan-in task that synthesizes the twelve verified reviews."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Mapping

from clef_sdk.model import (
    ArtifactBinding,
    ArtifactKind,
    ArtifactSpec,
    DomainContract,
    EffectKind,
    EffectPolicy,
    EffectRule,
    FrozenDict,
    Prompt,
    PromptRole,
    ResourcePolicy,
    RetryWorkspaceStrategy,
    SessionTask,
    TaskInput,
    VerifierSpec,
)

from .definitions import (
    FINAL_DECISION_SCHEMA,
    REVIEW_DEFINITIONS,
    ReviewInputs,
)


def build_final_report_task(
    review_tasks: Mapping[str, SessionTask],
    direct_inputs: ReviewInputs,
    workfolder: Path,
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    """Build the final report node and all incoming review bindings."""

    workfolder = Path(workfolder).expanduser().resolve(strict=False)
    task_id = "synthesize-final-review"
    bindings: list[ArtifactBinding] = []
    task_inputs: dict[str, TaskInput] = {
        name: value for name, value in direct_inputs.as_mapping().items()
    }
    contract_inputs: dict[str, ArtifactKind] = {
        "manuscript_md": ArtifactKind.TEXT,
        "manuscript_pdf": ArtifactKind.FILE,
        "review_context": ArtifactKind.JSON,
    }
    for item in REVIEW_DEFINITIONS:
        source_task = review_tasks[item.task_id]
        for output_name in ("report", "findings"):
            input_name = f"review_{item.review_id}_{output_name}"
            binding = ArtifactBinding(
                source_task_id=source_task.id,
                output_name=output_name,
                target_task_id=task_id,
                input_name=input_name,
            )
            task_inputs[input_name] = binding
            contract_inputs[input_name] = (
                ArtifactKind.TEXT
                if output_name == "report"
                else ArtifactKind.JSON
            )
            bindings.append(binding)

    stable_workspace = workfolder / "Report"
    report_path = stable_workspace / "final-review-report.md"
    decision_path = stable_workspace / "decision.json"
    ids_and_titles = [
        {
            "review_id": item.review_id,
            "dimension": item.slug,
            "title": item.title,
        }
        for item in REVIEW_DEFINITIONS
    ]
    instruction = f"""综合 12 份已经通过 clef 验证的审查结果，生成一份可供编辑和
作者使用的最终审稿报告。原始 manuscript_md/manuscript_pdf 只用于解决审查间
冲突或复核关键证据；不得新增无来源的事实、文献或实验结论。

先逐份读取 review_01…review_12 的 report 与 findings，建立 issue_id 去重表。
同一根因被多个维度发现时合并行动项，但保留全部 source_issue_ids；审查意见
冲突时在 conflicts 中明确双方、证据与采用的解释。建议等级必须由证据严重度、
结论稳健性和可修复性共同决定，不能简单投票。

输出两个且仅两个文件：
1. {report_path}：不少于 3000 个可见文字/数字字符的中文最终报告，依次包含
   “编辑摘要、稿件概述、总体建议、主要优点、阻断性问题、主要修改、次要修改、
   十二维度结论表、证据与问题映射、按优先级的修订清单、审查局限”。所有关键
   行动项都必须标注原始 issue_id；十二维度结论表必须覆盖下列 12 个 ID。
2. {decision_path}：严格 JSON，不带 Markdown fence，不添加 schema 外字段。
   dimension_reviews 必须按 01–12 排序且每个恰好一次；其 verdict 必须与对应
   findings 一致。priority_actions.priority 必须从 1 连续递增。

冻结维度：
{json.dumps(ids_and_titles, ensure_ascii=False, separators=(",", ":"))}

decision.json Schema：
{json.dumps(FINAL_DECISION_SCHEMA, ensure_ascii=False, sort_keys=True, separators=(",", ":"))}

recommendation 只能是 accept、minor_revision、major_revision、reject 或
unable_to_assess。不要把疑似伦理信号写成已证实的不端结论。完成后提交
SUCCEEDED AgentReport，声明 final_report 与 decision 两个 Artifact；不得修改
输入、不得联网、不得创建其他文件。
"""
    outputs = {
        "final_report": ArtifactSpec(
            name="final_report",
            description="综合十二维审查的中文最终报告",
            kind=ArtifactKind.TEXT,
            path="final-review-report.md",
        ),
        "decision": ArtifactSpec(
            name="decision",
            description="最终建议、维度覆盖与修订优先级",
            kind=ArtifactKind.JSON,
            path="decision.json",
        ),
    }
    task = SessionTask(
        id=task_id,
        domain_function="manuscript.review.synthesize.v1",
        prompts=(
            Prompt(
                PromptRole.POLICY,
                "只综合已验证输入；区分稿件事实、外部规范和审稿判断；每个否定性"
                "判断都必须可追溯到 issue_id 与稿件定位；保持专业、具体、可执行。",
                name="synthesis-evidence-policy",
                priority=100,
            ),
            Prompt(
                PromptRole.INSTRUCTION,
                instruction,
                name="final-review-report",
                priority=50,
            ),
        ),
        inputs=FrozenDict[TaskInput](task_inputs),
        outputs=FrozenDict[ArtifactSpec](outputs),
        contract=DomainContract(
            inputs=FrozenDict[ArtifactKind](contract_inputs),
            outputs=FrozenDict[ArtifactKind](
                {
                    "final_report": ArtifactKind.TEXT,
                    "decision": ArtifactKind.JSON,
                }
            ),
            effects=EffectPolicy(
                allowed=(
                    EffectRule(EffectKind.SHELL),
                    EffectRule(EffectKind.CREATE, "final-review-report.md"),
                    EffectRule(EffectKind.MODIFY, "final-review-report.md"),
                    EffectRule(EffectKind.CREATE, "decision.json"),
                    EffectRule(EffectKind.MODIFY, "decision.json"),
                ),
            ),
            resources=ResourcePolicy(
                max_attempts=2,
                retry_workspace_strategy=RetryWorkspaceStrategy.NEW,
            ),
            verifiers=(
                VerifierSpec(
                    "min_text_length",
                    FrozenDict[Any](
                        {"output": "final_report", "minimum": 3000}
                    ),
                ),
                VerifierSpec(
                    "json_schema",
                    FrozenDict[Any](
                        {
                            "output": "decision",
                            "schema": FINAL_DECISION_SCHEMA,
                        }
                    ),
                ),
                VerifierSpec(
                    "final_review_consistency",
                    FrozenDict[Any](
                        {
                            "report_output": "final_report",
                            "decision_output": "decision",
                        }
                    ),
                ),
            ),
        ),
        metadata=FrozenDict[Any](
            {
                "workspace_subdir": "Report",
                "stage": 2000,
                "topo_rank": 1,
                "order": 1,
                "output_ranks": {"final_report": 100, "decision": 200},
            }
        ),
    )
    return task, tuple(bindings)


__all__ = ["build_final_report_task"]
