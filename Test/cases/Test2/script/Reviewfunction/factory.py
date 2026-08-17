"""Small common factory used by each focused review function."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from clef_sdk.model import (
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

from .definitions import ReviewDefinition, ReviewInputs, review_findings_schema


PACKAGE_ROOT = Path(__file__).resolve().parent
PROMPTS_ROOT = PACKAGE_ROOT / "prompts"
REFERENCES_ROOT = PACKAGE_ROOT / "references"


def _read_text(path: Path) -> str:
    try:
        value = path.read_text(encoding="utf-8").strip()
    except OSError as error:
        raise RuntimeError(f"cannot load review material {path}: {error}") from error
    if not value:
        raise ValueError(f"review material is empty: {path}")
    return value


def _reference_context(item: ReviewDefinition) -> str:
    sections = []
    for file_name in item.reference_files:
        path = REFERENCES_ROOT / file_name
        sections.append(f"<!-- local-reference: {file_name} -->\n{_read_text(path)}")
    return "\n\n---\n\n".join(sections)


def build_review_task(
    item: ReviewDefinition,
    inputs: ReviewInputs,
    workfolder: Path,
) -> SessionTask:
    """Build one evidence-grounded review task."""

    workfolder = Path(workfolder).expanduser().resolve(strict=False)
    relative_workspace = Path("Reviewprocess") / item.workspace_name
    stable_workspace = workfolder / relative_workspace
    report_path = stable_workspace / "review.md"
    findings_path = stable_workspace / "findings.json"
    schema = review_findings_schema(item)

    common_policy = _read_text(PROMPTS_ROOT / "00_common.md")
    focus = _read_text(PROMPTS_ROOT / item.prompt_file)
    references = _reference_context(item)
    instruction = f"""你负责第 {item.review_id} 轮“{item.title}”。

审查输入：
- manuscript_md：MinerU 提取的全文 Markdown，是文本证据主来源；
- manuscript_pdf：原始 PDF，仅用于核对版式、公式、图表或 OCR 疑点；
- review_context：目标 venue、稿件类型与运行上下文。目标 venue 未指定时，
  不得虚构具体期刊硬性要求。

维度任务：
{focus}

请写两个且仅两个输出：
1. {report_path}：中文审查报告，至少 800 个可见文字/数字字符。依次包含
   “审查结论、核查范围、优点、问题清单、给作者的问题、审查局限”。
   每个问题使用稳定编号 {item.review_id}-I01、{item.review_id}-I02……
   并给出正文 section、PDF page（无法可靠确定时写“未知”）和可检索 anchor。
2. {findings_path}：严格 JSON，不得带 Markdown fence，不得添加 schema 外字段。
   issue_id 必须与报告一致并以 {item.review_id}-I 开头。没有问题时 issues 可为空，
   但必须说明实际核查过什么；证据不足时用 not_assessable，不得把猜测写成事实。

JSON Schema（字段名必须逐字遵守）：
{json.dumps(schema, ensure_ascii=False, sort_keys=True, separators=(",", ":"))}

完成写入后，按 Clef SDK 请求提交 SUCCEEDED AgentReport，并声明 report
和 findings 两个 Artifact。不得修改任何输入文件，不得联网，不得创建其他文件。
"""

    effects = EffectPolicy(
        allowed=(
            EffectRule(EffectKind.SHELL),
            EffectRule(EffectKind.CREATE, "review.md"),
            EffectRule(EffectKind.MODIFY, "review.md"),
            EffectRule(EffectKind.CREATE, "findings.json"),
            EffectRule(EffectKind.MODIFY, "findings.json"),
        ),
    )
    outputs = {
        "report": ArtifactSpec(
            name="report",
            description=f"{item.review_id} {item.title}人读报告",
            kind=ArtifactKind.TEXT,
            path="review.md",
        ),
        "findings": ArtifactSpec(
            name="findings",
            description=f"{item.review_id} {item.title}结构化发现",
            kind=ArtifactKind.JSON,
            path="findings.json",
        ),
    }
    return SessionTask(
        id=item.task_id,
        domain_function=f"manuscript.review.{item.slug}.v1",
        prompts=(
            Prompt(
                PromptRole.POLICY,
                common_policy,
                name="evidence-and-review-policy",
                priority=100,
            ),
            Prompt(
                PromptRole.CONTEXT,
                references,
                name="publisher-guidance",
                priority=70,
            ),
            Prompt(
                PromptRole.INSTRUCTION,
                instruction,
                name=f"{item.slug}-review",
                priority=50,
            ),
        ),
        inputs=FrozenDict[TaskInput](inputs.as_mapping()),
        outputs=FrozenDict[ArtifactSpec](outputs),
        contract=DomainContract(
            inputs=FrozenDict[ArtifactKind](
                {
                    "manuscript_md": ArtifactKind.TEXT,
                    "manuscript_pdf": ArtifactKind.FILE,
                    "review_context": ArtifactKind.JSON,
                }
            ),
            outputs=FrozenDict[ArtifactKind](
                {
                    "report": ArtifactKind.TEXT,
                    "findings": ArtifactKind.JSON,
                }
            ),
            effects=effects,
            resources=ResourcePolicy(
                max_attempts=2,
                retry_workspace_strategy=RetryWorkspaceStrategy.NEW,
            ),
            verifiers=(
                VerifierSpec(
                    "min_text_length",
                    FrozenDict[Any](
                        {"output": "report", "minimum": 800}
                    ),
                ),
                VerifierSpec(
                    "json_schema",
                    FrozenDict[Any](
                        {"output": "findings", "schema": schema}
                    ),
                ),
                VerifierSpec(
                    "review_bundle_consistency",
                    FrozenDict[Any](
                        {
                            "report_output": "report",
                            "findings_output": "findings",
                            "review_id": item.review_id,
                            "dimension": item.slug,
                            "title": item.title,
                        }
                    ),
                ),
            ),
        ),
        metadata=FrozenDict[Any](
            {
                "workspace_subdir": relative_workspace.as_posix(),
                "stage": 1000,
                "topo_rank": 0,
                "order": item.ordinal,
                "review_id": item.review_id,
                "dimension": item.slug,
                "output_ranks": {"report": 100, "findings": 200},
            }
        ),
    )


__all__ = ["build_review_task"]
