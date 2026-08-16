"""Build the blind supplementary reconstruction Clef DAG."""

from __future__ import annotations

import hashlib
import json
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from clef_sdk.model import (
    ArtifactBinding,
    ArtifactKind,
    ArtifactRef,
    ArtifactSpec,
    DomainContract,
    EffectKind,
    EffectPolicy,
    EffectRule,
    FailurePolicy,
    FrozenDict,
    Prompt,
    PromptRole,
    ResourcePolicy,
    RetryWorkspaceStrategy,
    SessionTask,
    TaskInput,
    VerifierSpec,
    WorkflowPlan,
    WorkflowPolicies,
)

from .analysis import sha256_file
from .schemas import (
    ARTIFACT_MANIFEST_SCHEMA,
    ASSESSMENT_SCHEMA,
    EVIDENCE_LEDGER_SCHEMA,
    METHODS_INFERENCE_SCHEMA,
    THEORY_INFERENCE_SCHEMA,
    VALIDATION_REPORT_SCHEMA,
)


@dataclass(frozen=True, slots=True)
class ReproductionInputs:
    """Immutable direct inputs for the reconstruction DAG."""

    manuscript_pdf: ArtifactRef
    manuscript_md: ArtifactRef
    prior_review: ArtifactRef
    prior_decision: ArtifactRef
    benchmark_spec: ArtifactRef

    def as_mapping(self) -> dict[str, ArtifactRef]:
        """Return inputs under stable slot names."""

        return {
            "manuscript_pdf": self.manuscript_pdf,
            "manuscript_md": self.manuscript_md,
            "prior_review": self.prior_review,
            "prior_decision": self.prior_decision,
            "benchmark_spec": self.benchmark_spec,
        }


BLIND_INPUT_DIRECTORY = "0000_blind_inputs"
_BLIND_INPUT_SOURCES = {
    "manuscript.pdf": Path("Testarticle.pdf"),
    "manuscript.md": Path("review-work/Extractedmd/full.md"),
    "prior-review.md": Path("review-work/Report/final-review-report.md"),
    "prior-decision.json": Path("review-work/Report/decision.json"),
    "benchmark-spec.json": Path("reproduction/benchmark-spec.json"),
}


def prepare_blind_input_bundle(test2_root: Path, workfolder: Path) -> Path:
    """Materialize the exact source allowlist used by live and offline runs.

    The agent never receives ``Test2`` itself as a read root.  Instead, each
    run gets a content-addressed copy of only the manuscript/review inputs and
    the public policy benchmark.  Host verifier code and numerical goldens are
    deliberately excluded.  Existing bundle members are immutable:
    reusing a workfolder after a source changes is rejected rather than
    silently replacing evidence beneath an old run.
    """

    test2_root = test2_root.expanduser().resolve(strict=True)
    workfolder = workfolder.expanduser().resolve(strict=True)
    bundle = workfolder / BLIND_INPUT_DIRECTORY
    bundle.mkdir(parents=True, exist_ok=True)
    manifest_entries: list[dict[str, str]] = []
    for relative_target, relative_source in _BLIND_INPUT_SOURCES.items():
        source = (test2_root / relative_source).resolve(strict=True)
        if not source.is_relative_to(test2_root):
            raise ValueError(f"blind input source escapes Test2: {source}")
        target = (bundle / relative_target).resolve(strict=False)
        if not target.is_relative_to(bundle):
            raise ValueError(f"blind input target escapes bundle: {target}")
        target.parent.mkdir(parents=True, exist_ok=True)
        source_digest = sha256_file(source)
        if target.exists():
            if not target.is_file() or sha256_file(target) != source_digest:
                raise FileExistsError(
                    "blind input bundle is immutable and conflicts with "
                    f"current source: {target}"
                )
        else:
            temporary = target.with_name(f".{target.name}.tmp")
            shutil.copyfile(source, temporary)
            temporary.replace(target)
        manifest_entries.append(
            {
                "path": relative_target.replace("\\", "/"),
                "sha256": source_digest,
            }
        )
    manifest = {
        "schema_version": "1.0",
        "policy": "fixed-input-allowlist",
        "entries": manifest_entries,
    }
    manifest_path = bundle / "input-bundle.json"
    serialized = (
        json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    )
    if manifest_path.exists():
        if manifest_path.read_text(encoding="utf-8") != serialized:
            raise FileExistsError(
                "blind input manifest conflicts with the current allowlist"
            )
    else:
        temporary = manifest_path.with_name(".input-bundle.json.tmp")
        temporary.write_text(serialized, encoding="utf-8")
        temporary.replace(manifest_path)
    return bundle.resolve(strict=True)


COMMON_POLICY = """这是一个盲 supplementary 重建基准。

允许读取提供的论文、OCR、审稿产物和 benchmark spec。禁止联网，禁止打开 DOI
附件或任何外部 supplementary 副本，禁止读取 .env。每个陈述必须标为正文事实、
可推导结果、推断结构或不可识别字段。正文不足以识别的 ABAQUS/实验参数必须保持
unknown/BLOCKED，不得用常见默认值填空。只写 AgentRequest 声明的输出文件。
"""


def _artifact_ref(
    path: Path,
    *,
    description: str,
    kind: ArtifactKind,
    media_type: str,
) -> ArtifactRef:
    path = path.expanduser().resolve(strict=True)
    return ArtifactRef(
        uri=str(path),
        description=description,
        kind=kind,
        digest=f"sha256:{sha256_file(path)}",
        media_type=media_type,
    )


def build_reproduction_inputs(bundle_root: Path) -> ReproductionInputs:
    """Bind inputs from a previously materialized blind input bundle."""

    bundle_root = bundle_root.expanduser().resolve(strict=True)
    return ReproductionInputs(
        manuscript_pdf=_artifact_ref(
            bundle_root / "manuscript.pdf",
            description="immutable Test2 paper PDF",
            kind=ArtifactKind.FILE,
            media_type="application/pdf",
        ),
        manuscript_md=_artifact_ref(
            bundle_root / "manuscript.md",
            description="fixed MinerU main-article extraction",
            kind=ArtifactKind.TEXT,
            media_type="text/markdown",
        ),
        prior_review=_artifact_ref(
            bundle_root / "prior-review.md",
            description="previous verified multi-dimensional review",
            kind=ArtifactKind.TEXT,
            media_type="text/markdown",
        ),
        prior_decision=_artifact_ref(
            bundle_root / "prior-decision.json",
            description="previous verified review decision",
            kind=ArtifactKind.JSON,
            media_type="application/json",
        ),
        benchmark_spec=_artifact_ref(
            bundle_root / "benchmark-spec.json",
            description="frozen reproduction acceptance specification",
            kind=ArtifactKind.JSON,
            media_type="application/json",
        ),
    )


def _outputs_effects(file_names: tuple[str, ...]) -> EffectPolicy:
    rules = [EffectRule(EffectKind.SHELL)]
    for file_name in file_names:
        rules.extend(
            (
                EffectRule(EffectKind.CREATE, file_name),
                EffectRule(EffectKind.MODIFY, file_name),
            )
        )
    return EffectPolicy(allowed=tuple(rules))


def _base_task(
    *,
    task_id: str,
    domain_function: str,
    instruction: str,
    inputs: dict[str, TaskInput],
    input_kinds: dict[str, ArtifactKind],
    outputs: dict[str, ArtifactSpec],
    verifiers: tuple[VerifierSpec, ...],
    workspace_subdir: str,
    stage: int,
    topo_rank: int,
    order: int,
) -> SessionTask:
    return SessionTask(
        id=task_id,
        domain_function=domain_function,
        prompts=(
            Prompt(
                PromptRole.POLICY,
                COMMON_POLICY,
                name="blind-reconstruction-policy",
                priority=100,
            ),
            Prompt(
                PromptRole.INSTRUCTION,
                instruction,
                name=task_id,
                priority=50,
            ),
        ),
        inputs=FrozenDict[TaskInput](inputs),
        outputs=FrozenDict[ArtifactSpec](outputs),
        contract=DomainContract(
            inputs=FrozenDict[ArtifactKind](input_kinds),
            outputs=FrozenDict[ArtifactKind](
                {name: output.kind for name, output in outputs.items()}
            ),
            effects=_outputs_effects(
                tuple(
                    output.path
                    for output in outputs.values()
                    if output.path is not None
                )
            ),
            resources=ResourcePolicy(
                max_attempts=2,
                retry_workspace_strategy=RetryWorkspaceStrategy.NEW,
            ),
            verifiers=verifiers,
        ),
        metadata=FrozenDict[Any](
            {
                "workspace_subdir": workspace_subdir,
                "stage": stage,
                "topo_rank": topo_rank,
                "order": order,
                "output_ranks": {
                    name: (index + 1) * 100 for index, name in enumerate(outputs)
                },
            }
        ),
    )


def _evidence_task(inputs: ReproductionInputs) -> SessionTask:
    outputs = {
        "evidence_report": ArtifactSpec(
            name="evidence_report",
            description="human-readable SI dependency inventory",
            kind=ArtifactKind.TEXT,
            path="evidence-report.md",
        ),
        "evidence_ledger": ArtifactSpec(
            name="evidence_ledger",
            description="machine-readable SI dependency ledger",
            kind=ArtifactKind.JSON,
            path="evidence-ledger.json",
        ),
    }
    instruction = f"""核查 main article、prior review 和 benchmark spec，建立缺失
supplementary 的穷举证据账本。必须区分正文 Appendix A 与在线 SI，并至少定位：
Eq. (48) T_I...T_IV、多项式 b^1 结果、实验/FEA 方法、Fig. S1。
benchmark_spec.evidence_output_contract 是公开输出合同：dependency_id、
recoverability、anchor/scope 语义与 source digest 字段必须逐项满足；事实仍须从
绑定的正文与审查材料核对，不得使用外部 supplementary。

写 evidence-report.md 与 evidence-ledger.json。JSON 必须匹配：
{json.dumps(EVIDENCE_LEDGER_SCHEMA, ensure_ascii=False, separators=(",", ":"))}
报告至少 600 个可见文字/数字字符。完成后提交 SUCCEEDED AgentReport。
"""
    direct = inputs.as_mapping()
    return _base_task(
        task_id="inventory-supplement-evidence",
        domain_function="paper.reproduction.inventory.v1",
        instruction=instruction,
        inputs={name: value for name, value in direct.items()},
        input_kinds={
            "manuscript_pdf": ArtifactKind.FILE,
            "manuscript_md": ArtifactKind.TEXT,
            "prior_review": ArtifactKind.TEXT,
            "prior_decision": ArtifactKind.JSON,
            "benchmark_spec": ArtifactKind.JSON,
        },
        outputs=outputs,
        verifiers=(
            VerifierSpec(
                "min_text_length",
                FrozenDict[Any]({"output": "evidence_report", "minimum": 600}),
            ),
            VerifierSpec(
                "json_schema",
                FrozenDict[Any](
                    {
                        "output": "evidence_ledger",
                        "schema": EVIDENCE_LEDGER_SCHEMA,
                    }
                ),
            ),
            VerifierSpec(
                "evidence_ledger_consistency",
                FrozenDict[Any](
                    {
                        "output": "evidence_ledger",
                        "pdf_input": "manuscript_pdf",
                        "markdown_input": "manuscript_md",
                    }
                ),
            ),
        ),
        workspace_subdir="Evidence",
        stage=1000,
        topo_rank=0,
        order=1,
    )


def _inference_task(
    *,
    theory: bool,
    inputs: ReproductionInputs,
    evidence_task: SessionTask,
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    task_id = "infer-theory-supplement" if theory else "infer-methods-supplement"
    input_name = "evidence_ledger"
    binding = ArtifactBinding(
        source_task_id=evidence_task.id,
        output_name="evidence_ledger",
        target_task_id=task_id,
        input_name=input_name,
    )
    task_inputs: dict[str, TaskInput] = {
        "manuscript_pdf": inputs.manuscript_pdf,
        "manuscript_md": inputs.manuscript_md,
        "benchmark_spec": inputs.benchmark_spec,
        input_name: binding,
    }
    input_kinds = {
        "manuscript_pdf": ArtifactKind.FILE,
        "manuscript_md": ArtifactKind.TEXT,
        "benchmark_spec": ArtifactKind.JSON,
        input_name: ArtifactKind.JSON,
    }
    if theory:
        outputs = {
            "theory_report": ArtifactSpec(
                name="theory_report",
                description="derived analytical supplementary material",
                kind=ArtifactKind.TEXT,
                path="theory-inference.md",
            ),
            "theory_inference": ArtifactSpec(
                name="theory_inference",
                description="structured theory derivation and unknowns",
                kind=ArtifactKind.JSON,
                path="theory-inference.json",
            ),
        }
        schema = THEORY_INFERENCE_SCHEMA
        report_output = "theory_report"
        json_output = "theory_inference"
        instruction = f"""只用正文方程恢复 SI 的理论职责。至少：
1. 从 Eq. (47) 推导 x=beta*S 下三阶算子，说明 T_I...T_III 的核空间
   与 T_IV 特解职责；不得声称自选基就是作者历史基。
2. 正文 561-595 行表明多项式 SI 缺失结果含 phi_(1)b(1)、
   U_2(2)b(1)、kappa_3_(1)b(1)。必须从 Eqs. (3), (34), (52)-(55),
   (66), (69) 恢复三个量，验证端点条件、phi'=kappa_3、
   U_2''=(pi/16)*tilde_F_2（L_S=1），并给出可验证的 mode-ratio 系数。
3. benchmark_spec.theory_output_contract 是公开输出合同，不是隐藏答案。
   section_id、status、validation_ids、required anchors/tokens 必须逐项满足；
   数学值仍须由正文独立推导，不得从 host verifier 或外部 SI 获取。

写 theory-inference.md（至少 800 个可见文字/数字字符）与
theory-inference.json。Schema：
{json.dumps(schema, ensure_ascii=False, separators=(",", ":"))}
"""
    else:
        outputs = {
            "methods_report": ArtifactSpec(
                name="methods_report",
                description="bounded experiment and FEA methods inference",
                kind=ArtifactKind.TEXT,
                path="methods-inference.md",
            ),
            "methods_inference": ArtifactSpec(
                name="methods_inference",
                description="known and missing experimental/FEA fields",
                kind=ArtifactKind.JSON,
                path="methods-inference.json",
            ),
        }
        schema = METHODS_INFERENCE_SCHEMA
        report_output = "methods_report"
        json_output = "methods_inference"
        instruction = f"""恢复正文能确认的 PET、几何、载荷、C3D8R/S4R 和
Fig. S1 对照设计；列出不能识别的 ABAQUS 版本、网格、本构、分析步、
样本量和测量不确定度。定量全场与实验统计必须标记 blocked。
benchmark_spec.methods_output_contract 是公开输出合同，不是隐藏答案。
confirmed_facts 的 ID、等价表示、正文锚点、Fig. S1 职责、缺失字段类别、
报告语义与禁止晋升的结论必须逐项满足；所有科学事实仍须由正文独立核对。

写 methods-inference.md（至少 800 个可见文字/数字字符）与
methods-inference.json。Schema：
{json.dumps(schema, ensure_ascii=False, separators=(",", ":"))}
"""
    task = _base_task(
        task_id=task_id,
        domain_function=(
            "paper.reproduction.infer_theory.v1"
            if theory
            else "paper.reproduction.infer_methods.v1"
        ),
        instruction=instruction,
        inputs=task_inputs,
        input_kinds=input_kinds,
        outputs=outputs,
        verifiers=(
            VerifierSpec(
                "min_text_length",
                FrozenDict[Any](
                    {
                        "output": report_output,
                        "minimum": 800,
                    }
                ),
            ),
            VerifierSpec(
                "json_schema",
                FrozenDict[Any]({"output": json_output, "schema": schema}),
            ),
            VerifierSpec(
                (
                    "theory_inference_consistency"
                    if theory
                    else "methods_inference_consistency"
                ),
                FrozenDict[Any](
                    {
                        "report_output": report_output,
                        "json_output": json_output,
                    }
                ),
            ),
        ),
        workspace_subdir="Inference/Theory" if theory else "Inference/Methods",
        stage=2000,
        topo_rank=1,
        order=1 if theory else 2,
    )
    return task, (binding,)


def _validation_task(
    inputs: ReproductionInputs,
    evidence_task: SessionTask,
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    task_id = "validate-paper-numerics"
    binding = ArtifactBinding(
        source_task_id=evidence_task.id,
        output_name="evidence_ledger",
        target_task_id=task_id,
        input_name="evidence_ledger",
    )
    outputs = {
        "validation_markdown": ArtifactSpec(
            name="validation_markdown",
            description="human-readable independent numerical checks",
            kind=ArtifactKind.TEXT,
            path="validation-report.md",
        ),
        "validation_report": ArtifactSpec(
            name="validation_report",
            description="machine-readable independent numerical checks",
            kind=ArtifactKind.JSON,
            path="validation-report.json",
        ),
    }
    instruction = f"""执行正文数值闭环。blind input 中不提供 host verifier
或 golden 数值程序；你必须仅由正文方程自行推导，并可用一次性 inline Python
数值计算（不得联网或读取 bundle 外路径）。覆盖 b_max、Fig.5 截断误差、
Fig.8 三形状 mode ratio、polynomial 的 phi/U2/kappa3 三个 SI 缺失量、
Fig.10 inverse design、预拉伸换算和直梁极限。不得手工改 PASS/FAIL。
benchmark_spec.numeric_output_contract 公开了 check ID、状态、输出字段、
证据锚点和容差语义；check 顺序与 title/interpretation 文案不影响验收，
但 required observed/expected 字段必须由正文方程独立计算。host verifier 会
用独立实现重算数值，不会把 golden 值提供给你。
validation-report.json Schema：
{json.dumps(VALIDATION_REPORT_SCHEMA, ensure_ascii=False, separators=(",", ":"))}
"""
    task = _base_task(
        task_id=task_id,
        domain_function="paper.reproduction.validate_numerics.v1",
        instruction=instruction,
        inputs={
            "manuscript_pdf": inputs.manuscript_pdf,
            "manuscript_md": inputs.manuscript_md,
            "benchmark_spec": inputs.benchmark_spec,
            "evidence_ledger": binding,
        },
        input_kinds={
            "manuscript_pdf": ArtifactKind.FILE,
            "manuscript_md": ArtifactKind.TEXT,
            "benchmark_spec": ArtifactKind.JSON,
            "evidence_ledger": ArtifactKind.JSON,
        },
        outputs=outputs,
        verifiers=(
            VerifierSpec(
                "min_text_length",
                FrozenDict[Any](
                    {
                        "output": "validation_markdown",
                        "minimum": 800,
                    }
                ),
            ),
            VerifierSpec(
                "json_schema",
                FrozenDict[Any](
                    {
                        "output": "validation_report",
                        "schema": VALIDATION_REPORT_SCHEMA,
                    }
                ),
            ),
            VerifierSpec(
                "numeric_reproduction_consistency",
                FrozenDict[Any](
                    {
                        "output": "validation_report",
                        "pdf_input": "manuscript_pdf",
                        "markdown_input": "manuscript_md",
                    }
                ),
            ),
        ),
        workspace_subdir="Validation",
        stage=2000,
        topo_rank=1,
        order=3,
    )
    return task, (binding,)


def _final_task(
    inputs: ReproductionInputs,
    upstream: tuple[SessionTask, ...],
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    task_id = "synthesize-inferred-supplement"
    task_inputs: dict[str, TaskInput] = {
        "manuscript_pdf": inputs.manuscript_pdf,
        "manuscript_md": inputs.manuscript_md,
        "benchmark_spec": inputs.benchmark_spec,
    }
    input_kinds: dict[str, ArtifactKind] = {
        "manuscript_pdf": ArtifactKind.FILE,
        "manuscript_md": ArtifactKind.TEXT,
        "benchmark_spec": ArtifactKind.JSON,
    }
    bindings: list[ArtifactBinding] = []
    for source_task in upstream:
        for output_name, output_spec in source_task.outputs.items():
            input_name = f"{source_task.id}_{output_name}".replace("-", "_")
            binding = ArtifactBinding(
                source_task_id=source_task.id,
                output_name=output_name,
                target_task_id=task_id,
                input_name=input_name,
            )
            task_inputs[input_name] = binding
            input_kinds[input_name] = output_spec.kind
            bindings.append(binding)
    outputs = {
        "inferred_supplement": ArtifactSpec(
            name="inferred_supplement",
            description="blind inferred supplementary materials",
            kind=ArtifactKind.TEXT,
            path="inferred-supplement.md",
        ),
        "assessment": ArtifactSpec(
            name="assessment",
            description="bounded reproduction assessment",
            kind=ArtifactKind.JSON,
            path="reproduction-assessment.json",
        ),
        "artifact_manifest": ArtifactSpec(
            name="artifact_manifest",
            description="digest manifest of verified reproduction artifacts",
            kind=ArtifactKind.JSON,
            path="artifact-manifest.json",
        ),
    }
    instruction = f"""综合所有已验证上游 Artifact，写：
1. inferred-supplement.md：至少 3000 个可见文字/数字字符，分证据账本、理论 S1/S2、
   实验/FEA S3-S5、数值验证、不可识别边界。不得把 BLOCKED 写成已复现。
2. reproduction-assessment.json：Schema
{json.dumps(ASSESSMENT_SCHEMA, ensure_ascii=False, separators=(",", ":"))}
3. artifact-manifest.json：列出证据、理论、方法、验证、supplement、assessment
   的发布相对路径和 sha256 digest；role/path 必须严格使用
   benchmark_spec.final_output_contract.manifest_slots；
   verification 固定 clef_verified。Schema
{json.dumps(ARTIFACT_MANIFEST_SCHEMA, ensure_ascii=False, separators=(",", ":"))}

benchmark_spec.final_output_contract 是公开的最终输出合同。按其中 topic、
blind-source declaration、validated_claims_policy、blocked_claims_policy、
blocked required-input 类别、cross-check tokens、forbidden claims、
manifest_policy 和 manifest slots 逐项综合；validated_claims 可用 claim_id
或“claim_id: description”，但额外 claim 必须在合同中列出且有上游证据；
不得以标题措辞差异替代语义，也不得把 BLOCKED 晋升为复现成功。
最终结论必须是 verified partial reproduction，除非验证报告没有任何 BLOCKED。
"""
    task = _base_task(
        task_id=task_id,
        domain_function="paper.reproduction.synthesize.v1",
        instruction=instruction,
        inputs=task_inputs,
        input_kinds=input_kinds,
        outputs=outputs,
        verifiers=(
            VerifierSpec(
                "min_text_length",
                FrozenDict[Any](
                    {
                        "output": "inferred_supplement",
                        "minimum": 3000,
                    }
                ),
            ),
            VerifierSpec(
                "json_schema",
                FrozenDict[Any](
                    {
                        "output": "assessment",
                        "schema": ASSESSMENT_SCHEMA,
                    }
                ),
            ),
            VerifierSpec(
                "json_schema",
                FrozenDict[Any](
                    {
                        "output": "artifact_manifest",
                        "schema": ARTIFACT_MANIFEST_SCHEMA,
                    }
                ),
            ),
            VerifierSpec(
                "reproduction_bundle_consistency",
                FrozenDict[Any](
                    {
                        "supplement_output": "inferred_supplement",
                        "assessment_output": "assessment",
                        "manifest_output": "artifact_manifest",
                    }
                ),
            ),
        ),
        workspace_subdir="Report",
        stage=3000,
        topo_rank=2,
        order=1,
    )
    return task, tuple(bindings)


def build_reproduction_plan(
    test2_root: Path,
    workfolder: Path,
) -> WorkflowPlan:
    """Build the 1 -> 3 -> 1 reconstruction and validation DAG."""

    test2_root = test2_root.expanduser().resolve(strict=True)
    workfolder = workfolder.expanduser().resolve(strict=True)
    bundle_root = prepare_blind_input_bundle(test2_root, workfolder)
    inputs = build_reproduction_inputs(bundle_root)
    evidence = _evidence_task(inputs)
    theory, theory_bindings = _inference_task(
        theory=True,
        inputs=inputs,
        evidence_task=evidence,
    )
    methods, methods_bindings = _inference_task(
        theory=False,
        inputs=inputs,
        evidence_task=evidence,
    )
    validation, validation_bindings = _validation_task(inputs, evidence)
    final, final_bindings = _final_task(
        inputs,
        (evidence, theory, methods, validation),
    )
    tasks = {task.id: task for task in (evidence, theory, methods, validation, final)}
    identity = hashlib.sha256(
        (
            inputs.manuscript_pdf.digest
            + inputs.manuscript_md.digest
            + str(workfolder).casefold()
        ).encode("utf-8")
    ).hexdigest()[:20]
    return WorkflowPlan(
        id=f"test2-reproduction-{identity}",
        tasks=FrozenDict[SessionTask](tasks),
        bindings=(
            *theory_bindings,
            *methods_bindings,
            *validation_bindings,
            *final_bindings,
        ),
        policies=WorkflowPolicies(
            max_concurrency=3,
            failure_policy=FailurePolicy.SKIP_DEPENDENTS,
            fail_fast=False,
            max_subagent_depth=1,
            max_fan_out=16,
        ),
        outputs=FrozenDict[ArtifactBinding](
            {
                "inferred_supplement": ArtifactBinding(
                    source_task_id=final.id,
                    output_name="inferred_supplement",
                ),
                "assessment": ArtifactBinding(
                    source_task_id=final.id,
                    output_name="assessment",
                ),
                "artifact_manifest": ArtifactBinding(
                    source_task_id=final.id,
                    output_name="artifact_manifest",
                ),
            }
        ),
    )


__all__ = [
    "ReproductionInputs",
    "prepare_blind_input_bundle",
    "build_reproduction_inputs",
    "build_reproduction_plan",
]
