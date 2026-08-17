"""Deterministic scholarly content used by the offline reproduction adapter."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .analysis import (
    build_validation_report,
    polynomial_completion_diagnostics,
    render_validation_markdown,
    sha256_file,
)

BENCHMARK_ID = "test2-blind-supplement-reproduction"


def build_evidence_ledger(pdf_path: Path, extracted_markdown: Path) -> dict[str, Any]:
    """Build the blind SI dependency inventory."""

    return {
        "schema_version": "1.0",
        "benchmark_id": BENCHMARK_ID,
        "source_identity": {
            "pdf_digest": f"sha256:{sha256_file(pdf_path)}",
            "markdown_digest": (f"sha256:{sha256_file(extracted_markdown)}"),
            "doi": "10.1016/j.jmps.2017.10.012",
        },
        "policy": {
            "external_supplement_used": False,
            "external_network_used": False,
            "unknowns_must_remain_unknown": True,
        },
        "si_dependencies": [
            {
                "dependency_id": "SI-THEORY-OBLIQUE-BASIS",
                "main_text_anchor": "Eq. (48), PDF page 224",
                "confirmed_scope": (
                    "T_I, T_II and T_III span the homogeneous solution and "
                    "T_IV is the forced solution of Eq. (47)."
                ),
                "recoverability": "partially_derivable",
            },
            {
                "dependency_id": "SI-THEORY-POLYNOMIAL",
                "main_text_anchor": "Section 3.2 after Eq. (66)",
                "confirmed_scope": (
                    "The omitted b^1 displacement, twist and twisting-"
                    "curvature results for the polynomial ribbon."
                ),
                "recoverability": "derivable",
            },
            {
                "dependency_id": "SI-METHODS-EXPERIMENT-FEA",
                "main_text_anchor": "Section 3.3, PDF page 231",
                "confirmed_scope": (
                    "Experimental fabrication/measurement and ABAQUS model "
                    "details supporting Figs. 3 and 4."
                ),
                "recoverability": (
                    "partially_identifiable_but_insufficient_for_replication"
                ),
            },
            {
                "dependency_id": "SI-FIGURE-S1",
                "main_text_anchor": "Section 3.3, Fig. S1 citation",
                "confirmed_scope": (
                    "With-substrate versus without-substrate FEA for a "
                    "sinusoidal ribbon at b=0.15 and 40% compression."
                ),
                "recoverability": "partially_derivable",
            },
        ],
    }


def render_evidence_report(ledger: dict[str, Any]) -> str:
    """Render the evidence inventory."""

    rows = [
        (
            item["dependency_id"],
            item["main_text_anchor"],
            item["recoverability"],
        )
        for item in ledger["si_dependencies"]
    ]
    lines = [
        "# Test2 supplementary 证据账本",
        "",
        "科学事实只锚定到 Testarticle.pdf 及与其摘要绑定的 MinerU full.md；"
        "既有审稿产物仅用于发现候选缺口，公开 benchmark contract 仅约束输出"
        "接口。没有访问 DOI 附件、网络缓存或作者代码。Appendix A 位于论文"
        "正文第 237 页，不属于本任务所称的缺失 SI。",
        "",
        "| 依赖 | 正文锚点 | 可恢复性 |",
        "| --- | --- | --- |",
    ]
    lines.extend(f"| {item} | {anchor} | {status} |" for item, anchor, status in rows)
    lines.extend(
        [
            "",
            "## 已确认的边界",
            "",
            "正文能确认 SI 至少有四个职责：给出斜压缩 Eq. (47) 的解函数，"
            "列出多项式梁的一阶结果，补充实验/FEA 方法，并以 Fig. S1 比较"
            "含基底和不含基底的模型。正文不能确定 ABAQUS 版本、网格尺寸、"
            "基底本构、分析步、样本量、误差棒或作者采用的 T 函数归一化。"
            "这些字段必须保留为 unknown，不能用常见设置补空。",
            "",
            "## 复用既有审稿证据",
            "",
            "Test/cases/Test2/review-work/Reviewprocess/07_method、08_correctness、"
            "09_reproducibility 与 10_results 均独立指出同一 SI 缺口。"
            "其中 correctness 报告已做过若干手算核查，本 DAG 不直接信任其"
            "结论，而是由 host 数值节点重新计算并生成可机验 JSON。",
            "",
        ]
    )
    return "\n".join(lines)


def build_theory_inference() -> dict[str, Any]:
    """Return the machine-readable theory reconstruction."""

    return {
        "schema_version": "1.0",
        "benchmark_id": BENCHMARK_ID,
        "external_supplement_used": False,
        "sections": [
            {
                "section_id": "SI-THEORY-POLYNOMIAL",
                "status": "derived",
                "evidence": [
                    "Eqs. (34), (52)-(55), (65), (66), (71)",
                    "Section 3.2 delegates polynomial b^1 terms to SI",
                ],
                "reconstruction": (
                    "For x=S/L, F=1-12x^2+48x^4-64x^6, "
                    "K_1,b1=(24-576x^2+1920x^4)/L, and "
                    "m_1,0,b1=(4*pi^2*EI_2/L)*(F-16/35). Thus "
                    "g=-pi^2*F'(x)*cos(2*pi*x)+2*pi^3*(16/35-F(x))*"
                    "sin(2*pi*x), and kappa_3_(1)b(1)=(1+nu)*g/L. "
                    "The integration constant is zero by odd parity. "
                    "phi_(1)b(1) is the endpoint-anchored integral of kappa_3 "
                    "from Eq. (69). U_2(2)b(1) is the boundary-corrected "
                    "double integral of f2 in Eq. (54), equivalently "
                    "L*pi/16*[J(x)-J(1/2)]; for the polynomial, "
                    "sin(Theta)_(b1) vanishes at both endpoints."
                ),
                "validation_ids": ["NUM-FIG8-001", "NUM-BMAX-001"],
                "residual_unknowns": [
                    "The historical SI's algebraic formatting",
                    "Any author code used to generate the closed form",
                ],
            },
            {
                "section_id": "SI-THEORY-OBLIQUE-BASIS",
                "status": "operator_identified",
                "evidence": ["Eqs. (47)-(51)"],
                "reconstruction": (
                    "With x=beta*S, define L_gamma[y]=d/dx("
                    "y''-2*y'/x+(x^2+gamma)*y). T_I, T_II and T_III "
                    "must form a linearly independent basis of ker(L_gamma), "
                    "while T_IV is any particular solution of L_gamma[y]="
                    "beta^-3 times the right-hand side of Eq. (47). Boundary "
                    "conditions (44)-(45) plus normalization determine the "
                    "six constants after kappa_2, psi and U_1 are recovered."
                ),
                "validation_ids": ["LIMIT-OBLIQUE-001"],
                "residual_unknowns": [
                    "The authors' exact special-function basis",
                    "Basis ordering and normalization",
                ],
            },
        ],
    }


def render_theory_report(theory: dict[str, Any]) -> str:
    """Render the inferred analytical supplement."""

    del theory
    diagnostics = polynomial_completion_diagnostics()
    sample_rows = zip(
        diagnostics["sample_x"],
        diagnostics["kappa3"],
        diagnostics["phi"],
        diagnostics["u2"],
        strict=True,
    )
    lines = [
        "# 推导的理论 Supplement",
        "",
        "## S1. 多项式形状的一阶结果",
        "",
        "令 `x=S/L_S`，式 (66) 的多项式形状可展开为",
        "",
        "`F_p(x)=1-12x^2+48x^4-64x^6`。",
        "",
        "由式 (19)、(34) 在 b=0 处展开，得到",
        "",
        "- `K_1,b1=(24-576x^2+1920x^4)/L_S`；",
        "- `integral[-1/2,1/2] F_p dx=16/35`；",
        "- `integral[-1/2,1/2] x F_p dx=0`；",
        "- `m_1,0,b1=(4*pi^2*EI_2/L_S)*(F_p-16/35)`。",
        "",
        "把这些量代回正文式 (55)，并使用窄矩形截面"
        " `GJ=2EI_2/(1+nu)`，得到可执行的等价积分式：",
        "",
        "`kappa_3,(1)b(1)(x)=((1+nu)*pi^2/L_S)`",
        "",
        "`  * integral_0^x cos(2*pi*u)`",
        "",
        "`  * [24-576u^2+1920u^4-4*pi^2*(F_p(u)-16/35)] du.`",
        "",
        "等价地，定义",
        "",
        "`g(x)=-pi^2 F'_p(x) cos(2*pi*x)`",
        "",
        "`     +2*pi^3[16/35-F_p(x)] sin(2*pi*x)`，",
        "",
        "则 `kappa_3,(1)b(1)=(1+nu)g/L_S`。这个化简使奇偶性直接"
        "可见：g 与 kappa_3 为奇函数。",
        "",
        "被积函数为偶函数，因此 kappa_3 为奇函数，式 (55) 的均值修正"
        "常数为零。这是 SI 中缺失闭式多项式的数值等价物；它不依赖"
        "猜测的补充材料符号排版。",
        "",
        "但正文第 561-595 行委托给 SI 的不只有 kappa_3。依据式 (3)、"
        "(54)、(55) 与 (69)，还必须恢复端点锚定的"
        " `phi_(1)b(1)` 和边界修正双积分 `U_2(2)b(1)`。多项式"
        " `sin(Theta)_(b1)=F'_p` 在两端为零，因此式 (54) 的端点角"
        "项严格消失，而线性修正确保 U_2(-1/2)=U_2(1/2)=0。",
        "",
        "令 `tau=(1+nu)g+pi*k*sin(2*pi*x)`、"
        "`B(x)=integral_x^(1/2) tau(u)du`，则",
        "",
        "`f2=8*pi*(cos(4*pi*x)-1)*k`",
        "",
        "`   +(-4*pi*x+sin(4*pi*x))*k' +32*pi*cos(2*pi*x)*B`。",
        "",
        "再令"
        "`J(x)=integral_0^x (x-u)f2(u)du`，可写成"
        "`U_2(2)b(1)=L_S*pi/16*[J(x)-J(1/2)]`。",
        "",
        "在 `nu=0.39, L_S=EI_2=1` 下，三个缺失量的抽样值如下：",
        "",
        "| x | L_S kappa_3,b1 | phi_b1 | U_2,b1/L_S |",
        "| ---: | ---: | ---: | ---: |",
    ]
    lines.extend(
        f"| {x_value:.3f} | {kappa:.9f} | {phi:.9f} | {u2:.9f} |"
        for x_value, kappa, phi, u2 in sample_rows
    )
    lines.extend(
        [
            "",
            "验证分三类。端点和奇偶性是构造不变量；phi'=kappa_3 与"
            " `U_2''=(pi/16)*tilde_F_2`（L_S=1）是离散数值一致性检查；"
            "与生产均匀网格不同的自适应 Gauss-Kronrod 五点基准、正文"
            "式 (71) 及 arc 闭式是交叉检查。三组样点均在公开容差内；"
            "表中端点约 4.39e-6 的 kappa_3 是离散残差，理论端点值为零。",
            "",
            "对该曲线按式 (65) 积分，得到"
            " `R_mode/b=0.300591377`；正文式 (71) 的闭式系数"
            " `0.216252789*(1+nu)` 在 nu=0.39 时也是"
            " `0.300591377`。两条独立路径的误差小于 2e-7。",
            "",
            "## S2. 斜压缩解函数的可识别内容",
            "",
            "令 `x=beta*S`。式 (47) 左端严格缩放为",
            "",
            "`beta^3 * d/dx[y''-2y'/x+(x^2+gamma)y]`。",
            "",
            "因此可唯一推断的数学契约是：`T_I,T_II,T_III` 张成三阶齐次算子",
            "",
            "`L_gamma[y]=d/dx[y''-2y'/x+(x^2+gamma)y]`",
            "",
            "的核，`T_IV` 是相应右端的一个特解。随后依照式 (49)-(51)"
            "恢复弯曲曲率、内力、psi、扭角和位移，再由式 (44)-(45)"
            "及归一化确定常数。Eq. (45) 的 OCR 转录问题可直接回看本地 PDF"
            "解决，不属于 supplementary 缺失造成的不可识别性。",
            "",
            "这个函数基不是唯一的：任何非奇异线性组合都表示同一解空间。"
            "在缺少 SI 时，把某个自选 Frobenius/特殊函数基冠以作者原始"
            " `T_I...T_IV` 名称会制造不可证实的历史细节。本复现仅确认"
            "算子、解空间维数、特解职责和边界闭合关系；作者采用的基排序、"
            "归一化列为 `BLOCKED` 受阻项。",
            "",
        ]
    )
    return "\n".join(lines)


def build_methods_inference() -> dict[str, Any]:
    """Return confirmed and non-identifiable methods content."""

    return {
        "schema_version": "1.0",
        "benchmark_id": BENCHMARK_ID,
        "confirmed_facts": [
            {
                "fact_id": "EXP-MATERIAL",
                "value": "PET, thickness 40 um, E=3.5 GPa, nu=0.39",
                "anchor": "Section 3.3",
            },
            {
                "fact_id": "EXP-GEOMETRY",
                "value": "h:w:L_S=1:30:900",
                "anchor": "Section 3.3",
            },
            {
                "fact_id": "EXP-END-COMPRESSION",
                "value": "epsilon_app=30% for the three-shape comparison",
                "anchor": "Fig. 3 caption",
            },
            {
                "fact_id": "EXP-SHAPE-B",
                "value": "b=0.15, 0.25, 2*pi/3 for sinusoidal, polynomial, arc",
                "anchor": "Fig. 3 caption",
            },
            {
                "fact_id": "EXP-OBLIQUE",
                "value": "sinusoidal b=0.1, epsilon_app=30%, alpha=10 and 30 deg",
                "anchor": "Fig. 4 caption",
            },
            {
                "fact_id": "FEA-SOFTWARE",
                "value": "commercial ABAQUS; version not stated",
                "anchor": "Section 3.3",
            },
            {
                "fact_id": "FEA-ELEMENTS",
                "value": "C3D8R and S4R with refined meshes",
                "anchor": "Section 3.3",
            },
            {
                "fact_id": "FIG-S1-LOAD",
                "value": "sinusoidal b=0.15, epsilon_app=40%, prestrain about 66.7%",
                "anchor": "Fig. S1 description in Section 3.3",
            },
        ],
        "figure_s1": {
            "comparison": (
                "full model with elastomer substrate versus ribbon model "
                "without substrate"
            ),
            "parameters": [
                "sinusoidal ribbon",
                "b=0.15",
                "epsilon_app=40%",
                "approximately 66.7% substrate prestrain",
                "C3D8R and S4R elements",
            ],
            "reported_outcome": (
                "local bending appears near bonded regions, while the global "
                "3D ribbon deformation remains almost unchanged"
            ),
            "quantitative_reproduction": "blocked",
        },
        "missing_fields": [
            "ABAQUS version and analysis procedure",
            "element counts, mesh sizes and convergence study",
            "substrate dimensions and constitutive parameters",
            "contact/bond/tie definitions and loading amplitudes",
            "solver controls and imperfection seeding",
            "experimental sample size and repeats",
            "imaging/calibration/3D coordinate extraction protocol",
            "raw coordinates, uncertainty and error bars",
        ],
        "replication_status": "blocked_without_raw_methods_and_data",
    }


def render_methods_report(methods: dict[str, Any]) -> str:
    """Render the inferred methods supplement."""

    lines = [
        "# 推导的实验与 FEA Supplement",
        "",
        "## S3. 正文能够恢复的实验协议",
        "",
        "论文使用 40 um PET 薄膜（E=3.5 GPa, nu=0.39），代表性梁的"
        "几何比为 h:w:L_S=1:30:900。端对端三形状比较采用 30% 压缩，"
        "正弦、多项式、圆弧的 b 分别是 0.15、0.25、2*pi/3。斜压缩"
        "比较使用 b=0.1、30% 压缩以及 10 和 30 度加载角。",
        "",
        "正文说明薄膜被图形化后与预拉伸硅弹性体集成，但没有给出弹性体"
        "牌号、厚度、模量、预拉伸夹具、键合图案尺寸、对准公差、样本量、"
        "重复次数或三维坐标测量流程。因此这里能够恢复的是实验设计矩阵，"
        "不是可独立重复的制造 SOP。",
        "",
        "## S4. 正文能够恢复的 FEA 与 Fig. S1",
        "",
        "正文明确命名 ABAQUS、C3D8R、S4R 和 refined meshes。Fig. S1"
        " 必须是一组 b=0.15 正弦梁在 40% 压缩下的对照：一组显式包含"
        "弹性体基底，另一组省略基底。正文报告含基底模型只在键合区引入"
        "轻微弯曲，整体三维形状几乎不变。",
        "",
        "66.7% 预拉伸与约 40% 压缩之间不是拟合值。若预拉伸"
        " `p=(L_pre-L_0)/L_0`，释放相对预拉伸长度的压缩是"
        " `p/(1+p)`；若 66.7% 是 2/3 的四舍五入，则对应约 40%，"
        "取精确 p=2/3 时等于 0.4。该换算由 NUM-PRESTRAIN-001 重算。",
        "",
        "## S5. 不可从正文识别的字段",
        "",
    ]
    lines.extend(f"- {item}" for item in methods["missing_fields"])
    lines.extend(
        [
            "",
            "缺失字段会显著改变局部应力、屈曲路径和收敛行为。故本推导"
            "不填入“常用”ABAQUS 设置，也不生成伪造的 Fig. S1 数值曲线。"
            "定性图意和载荷换算可验证；全场差异、网格收敛与实验统计保持"
            " BLOCKED，直到获得 inp/cae、材料卡与原始测量。",
            "",
        ]
    )
    return "\n".join(lines)


def build_assessment(validation: dict[str, Any]) -> dict[str, Any]:
    """Build the final bounded reproduction assessment."""

    validated = [
        item["check_id"] for item in validation["checks"] if item["status"] == "PASS"
    ]
    return {
        "schema_version": "1.0",
        "benchmark_id": BENCHMARK_ID,
        "reproduction_status": "partial_reproduction",
        "external_supplement_used": False,
        "historical_identity_verified": False,
        "validated_claims": validated,
        "blocked_claims": [
            {
                "claim_id": "LIMIT-OBLIQUE-001",
                "reason": (
                    "The operator is identifiable but the authors' historical "
                    "basis normalization is not."
                ),
                "required_inputs": [
                    "publisher SI identifying the historical basis",
                    "author basis ordering and normalization metadata",
                ],
            },
            {
                "claim_id": "LIMIT-FEA-001",
                "reason": (
                    "No ABAQUS input, mesh, substrate material card or field "
                    "data are present."
                ),
                "required_inputs": [
                    "inp or equivalent full model",
                    "mesh convergence evidence",
                    "field output behind Fig. S1",
                ],
            },
            {
                "claim_id": "LIMIT-EXPERIMENT-001",
                "reason": (
                    "No raw coordinates, repeat counts, uncertainty or complete "
                    "fabrication protocol are present."
                ),
                "required_inputs": [
                    "raw 3D measurements",
                    "sample counts and uncertainty",
                    "complete substrate and bonding protocol",
                ],
            },
        ],
        "conclusion": (
            "The main article is sufficient to reproduce its geometric limits, "
            "the reported sinusoidal truncation errors, first-order mode-ratio "
            "coefficients and a candidate reconstruction of all three omitted "
            "polynomial b-order quantities that is consistent with the article "
            "(twisting curvature, endpoint-anchored twist angle, and in-plane "
            "displacement), strain "
            "conversion, straight-beam limit and inverse-design approximation. "
            "It is insufficient for a quantitative replay of oblique special "
            "functions as historically normalized, Fig. S1 ABAQUS fields or "
            "experimental statistics. The scientifically valid outcome is "
            "therefore a verified partial reproduction, not a claimed full one; "
            "historical identity with the publisher SI was not evaluated."
        ),
    }


def render_inferred_supplement(
    evidence_report: str,
    theory_report: str,
    methods_report: str,
    validation_markdown: str,
) -> str:
    """Assemble the bounded inferred supplementary document."""

    return "\n\n".join(
        [
            "# Inferred Supplementary Materials - blind reconstruction",
            (
                "> Artifact class: deterministic offline reference used to test "
                "the DAG, verifier and publication pipeline; it is not evidence "
                "of live-agent autonomy. Scientific facts are anchored to the "
                "main article, local review artifacts only locate candidate "
                "gaps, and the public benchmark contract defines output shape. "
                "The publisher supplementary file was not used. “Inferred” does "
                "not mean historically identical."
            ),
            evidence_report,
            theory_report,
            methods_report,
            validation_markdown,
            "\n".join(
                [
                    "## 最终可证结论",
                    "",
                    "双扰动理论的若干中心数值关系可以在没有 SI 的情况下闭环，"
                    "尤其是 SI 缺失的三个多项式 b 阶量可以从正文一般式构造出"
                    "候选重建，并通过正文闭式模态比、自适应积分基准和离散"
                    "残差进行一致性与交叉检查；这不证明其排版或表达与作者"
                    "历史 SI 完全相同。",
                    "",
                    "相反，Fig. S1 的“几乎不变”没有数字阈值，实验图没有原始"
                    "坐标或不确定度，斜压缩函数基也存在等价基自由度。框架必须"
                    "把这些项保持为受阻证据，而不是让主 agent 用看似合理的"
                    "网格、材料或特殊函数填空。这一边界正是 Test2 作为 agent"
                    "工作流回归案例的核心验收点。",
                ]
            ),
        ]
    )


def json_text(value: dict[str, Any]) -> str:
    """Return stable, readable JSON."""

    return json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def build_all_offline_content(
    pdf_path: Path, extracted_markdown: Path
) -> dict[str, Any]:
    """Build all semantic documents needed by the fake adapter."""

    evidence = build_evidence_ledger(pdf_path, extracted_markdown)
    theory = build_theory_inference()
    methods = build_methods_inference()
    validation = build_validation_report(pdf_path, extracted_markdown)
    evidence_report = render_evidence_report(evidence)
    theory_report = render_theory_report(theory)
    methods_report = render_methods_report(methods)
    validation_markdown = render_validation_markdown(validation)
    supplement = render_inferred_supplement(
        evidence_report,
        theory_report,
        methods_report,
        validation_markdown,
    )
    assessment = build_assessment(validation)
    return {
        "evidence": evidence,
        "evidence_report": evidence_report,
        "theory": theory,
        "theory_report": theory_report,
        "methods": methods,
        "methods_report": methods_report,
        "validation": validation,
        "validation_markdown": validation_markdown,
        "supplement": supplement,
        "assessment": assessment,
    }


__all__ = [
    "BENCHMARK_ID",
    "build_all_offline_content",
    "build_assessment",
    "build_evidence_ledger",
    "build_methods_inference",
    "build_theory_inference",
    "json_text",
    "render_evidence_report",
    "render_inferred_supplement",
    "render_methods_report",
    "render_theory_report",
]
