# ruff: noqa: RUF001
"""Build the Pelican Ride agentic design/build/review/package DAG."""

from __future__ import annotations

import hashlib
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

COMMON_POLICY = """你正在执行 Clef SDK 的 Pelican Ride 算例。

所有输入 Artifact 都是只读证据。只能在 AgentRequest 声明的输出目录内创建或
修改文件，不得修改任何上游 Artifact、仓库文件、用户配置或 .env。允许为设计
灵感访问公开网页，允许调用本机 dotnet 和必要的系统工具，但不得下载或复用受
版权约束的图像、字体、音频、代码或二进制资产。

最终产品必须由项目自身的 C#/WPF 程序化矢量、渐变、粒子和系统能力构成；运行时
不得联网，不得依赖第三方 NuGet 包、DLL 或 loose assets。每个任务都要先检查
绑定的 benchmark.json，再执行实质验证。完成时只申报 AgentRequest 中具名的
Artifact，并提交严格的 SUCCEEDED AgentReport；不要用文字声称替代文件产物。
"""


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _benchmark_ref(case_root: Path) -> ArtifactRef:
    path = (case_root / "benchmark.json").resolve(strict=True)
    return ArtifactRef(
        uri=str(path),
        description="frozen Pelican Ride product and acceptance contract",
        kind=ArtifactKind.JSON,
        digest=f"sha256:{_sha256(path)}",
        media_type="application/json",
    )


def _directory_effects(
    output_path: str,
    *,
    network: bool = False,
) -> EffectPolicy:
    rules: list[EffectRule] = [EffectRule(EffectKind.SHELL)]
    if network:
        rules.append(EffectRule(EffectKind.NETWORK))
    for kind in (
        EffectKind.CREATE,
        EffectKind.MODIFY,
        EffectKind.MOVE,
        EffectKind.DELETE,
    ):
        rules.extend(
            (
                EffectRule(kind, output_path),
                EffectRule(kind, f"{output_path}/*"),
                EffectRule(kind, f"{output_path}/**"),
                EffectRule(kind, f"{output_path}/**/*"),
            )
        )
    return EffectPolicy(allowed=tuple(rules))


def _task(
    *,
    task_id: str,
    domain_function: str,
    instruction: str,
    inputs: dict[str, TaskInput],
    input_kinds: dict[str, ArtifactKind],
    output_name: str,
    output_path: str,
    description: str,
    verifier: str,
    workspace_subdir: str,
    stage: int,
    topo_rank: int,
    order: int,
    network: bool = False,
) -> SessionTask:
    output = ArtifactSpec(
        name=output_name,
        description=description,
        kind=ArtifactKind.DIRECTORY,
        path=output_path,
    )
    return SessionTask(
        id=task_id,
        domain_function=domain_function,
        prompts=(
            Prompt(
                role=PromptRole.POLICY,
                content=COMMON_POLICY,
                name="pelican-case-policy",
                priority=100,
            ),
            Prompt(
                role=PromptRole.INSTRUCTION,
                content=instruction,
                name=task_id,
                priority=50,
            ),
        ),
        inputs=FrozenDict[TaskInput](inputs),
        outputs=FrozenDict[ArtifactSpec]({output_name: output}),
        contract=DomainContract(
            inputs=FrozenDict[ArtifactKind](input_kinds),
            outputs=FrozenDict[ArtifactKind](
                {output_name: ArtifactKind.DIRECTORY}
            ),
            effects=_directory_effects(output_path, network=network),
            resources=ResourcePolicy(
                max_attempts=2,
                retry_workspace_strategy=RetryWorkspaceStrategy.NEW,
            ),
            verifiers=(
                VerifierSpec(
                    name=verifier,
                    parameters=FrozenDict[Any]({"output": output_name}),
                ),
            ),
        ),
        metadata=FrozenDict[Any](
            {
                "workspace_subdir": workspace_subdir,
                "stage": stage,
                "topo_rank": topo_rank,
                "order": order,
                "output_ranks": {output_name: 100},
            }
        ),
    )


def _design_task(benchmark: ArtifactRef) -> SessionTask:
    return _task(
        task_id="compose-vector-art-direction",
        domain_function="pelican.design.vector_art.v1",
        instruction="""保留 Simon Willison 原始 prompt 的难点，先完成专业视觉系统。
在 design-bundle 中只创建：

1. creative-brief.md：不少于 1200 个可见字符，明确构图网格、主体比例、深度层、
   HUD、动效节奏、响应式安全区、可读性和减少动态模式。必须逐项解释鹈鹕的大喙/
   喉囊、座姿、腿脚、车架、双轮、曲柄和踏板如何形成正确空间关系。
2. pelican-reference.svg：完整独立 SVG，viewBox 至少 1200x700；画面是一只可辨认
   的鹈鹕真正坐在自行车上，含双轮/辐条/车架/车把/鞍座/曲柄/踏板/腿脚，另有
   海岸背景与简洁 HUD 示意。元素或 group id 使用有语义的英文名称。不得嵌入
   base64、外链、脚本或栅格图片。
3. palette.json：严格 JSON，包含 background、ocean、pelican、bicycle、accent、
   ui 六组，每组至少 2 个 #RRGGBB 色值，并说明 contrast_usage。

先参考公开 pelican-bicycle 历代结果理解常见空间错误，再原创设计；不得复制现成
SVG。用 XML/结构检查确认 SVG 有效，并核对 palette。完成后只申报 design_bundle。
""",
        inputs={"benchmark": benchmark},
        input_kinds={"benchmark": ArtifactKind.JSON},
        output_name="design_bundle",
        output_path="design-bundle",
        description="vector reference, visual system and palette",
        verifier="design_bundle",
        workspace_subdir="0100_design",
        stage=1000,
        topo_rank=0,
        order=1,
        network=True,
    )


def _game_spec_task(benchmark: ArtifactRef) -> SessionTask:
    return _task(
        task_id="compose-gameplay-architecture",
        domain_function="pelican.design.gameplay_architecture.v1",
        instruction="""把 benchmark 转成可直接实现、可测试的游戏与软件规范。
在 game-spec-bundle 中只创建：

1. architecture.md：不少于 1400 个可见字符。定义 WPF 单进程架构、固定/半固定
   timestep、CompositionTarget.Rendering、输入边沿、状态机、对象池、碰撞、
   公平生成器、难度曲线、存档、程序化音效、DPI/缩放、诊断模式与故障边界。
   明确自行车轮速、曲柄、脚、身体 bob 的相位关系。不得采用第三方包。
2. acceptance.json：严格 JSON，根对象包含 product、controls、features、
   visual_requirements、technical_requirements、diagnostics、acceptance_tests。
   acceptance_tests 至少 18 项，每项有 id、kind、requirement、evidence、blocking；
   覆盖骑乘构图、两轮拓扑、动画不出界、所有按键、状态机、碰撞公平性、60 FPS、
   大帧 delta clamp、持久高分、静音、减少动态、高 DPI、无网运行、单文件发布、
   --smoke-test 与 --render-preview。

这不是泛泛建议，而是下游实现与审查的合同。检查 JSON 可解析、ID 唯一且所有
benchmark 要求有对应测试。完成后只申报 game_spec_bundle。
""",
        inputs={"benchmark": benchmark},
        input_kinds={"benchmark": ArtifactKind.JSON},
        output_name="game_spec_bundle",
        output_path="game-spec-bundle",
        description="implementation architecture and acceptance matrix",
        verifier="game_spec_bundle",
        workspace_subdir="0110_game_spec",
        stage=1000,
        topo_rank=0,
        order=2,
        network=True,
    )


def _implementation_task(
    benchmark: ArtifactRef,
    design: SessionTask,
    game_spec: SessionTask,
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    task_id = "build-playable-prototype"
    design_binding = ArtifactBinding(
        source_task_id=design.id,
        output_name="design_bundle",
        target_task_id=task_id,
        input_name="design_bundle",
    )
    spec_binding = ArtifactBinding(
        source_task_id=game_spec.id,
        output_name="game_spec_bundle",
        target_task_id=task_id,
        input_name="game_spec_bundle",
    )
    task = _task(
        task_id=task_id,
        domain_function="pelican.implementation.wpf_game.v1",
        instruction="""依据 benchmark、vector reference 和 gameplay contract 实现
完整可玩的第一版。只在 source-bundle 中写源码；禁止把 bin、obj、.vs、NuGet
缓存或临时发布目录留在输出中。

硬约束：
- C#、WPF、net8.0-windows、win-x64、OutputType=WinExe，禁用 trimming/AOT；
- 无 PackageReference、WebView、外部引擎、网络、loose runtime assets；
- csproj 内配置 SelfContained、PublishSingleFile、IncludeNativeLibrariesForSelfExtract、
  EnableCompressionInSingleFile、PublishReadyToRun、DebugSymbols=false；
- 所有画面由 DrawingContext/Geometry/Brush/Shape 等程序化矢量绘制；参考 SVG，
  不在运行时加载 SVG；
- 主循环 clamp delta，具有清晰的 Start/Playing/Paused/GameOver 状态；
- 实现 benchmark 全部按键、玩法、HUD、碰撞、难度、高分、静音、减少动态与全屏；
- 轮、辐条、曲柄、踏板、腿脚和身体动画必须随真实速度同步，鹈鹕保持在鞍座上；
- 至少 4 层视差、昼暮变化、收集/碰撞/加速粒子与精致 overlay；
- 设置写 LOCALAPPDATA/PelicanRide，EXE 邻目录零写入；
- 实现 `--smoke-test`（完全非交互、确定性、自检后退出码 0）、
  `--render-preview <absolute.png>`（渲染 1440x900 代表帧并退出）和 `--version`。

优先把渲染、模拟、输入/状态、音效/设置、诊断拆成可维护文件，避免一个巨型
code-behind。使用本机 dotnet 直接在 source-bundle 内 restore/build 并实际运行
smoke 与 preview；把发现的编译或运行问题全部修好。完成后递归清除 bin、obj、
publish 等构建目录，只申报 source_bundle。
""",
        inputs={
            "benchmark": benchmark,
            "design_bundle": design_binding,
            "game_spec_bundle": spec_binding,
        },
        input_kinds={
            "benchmark": ArtifactKind.JSON,
            "design_bundle": ArtifactKind.DIRECTORY,
            "game_spec_bundle": ArtifactKind.DIRECTORY,
        },
        output_name="source_bundle",
        output_path="source-bundle",
        description="buildable WPF Pelican Ride source tree",
        verifier="wpf_source_bundle",
        workspace_subdir="0200_implementation",
        stage=2000,
        topo_rank=1,
        order=1,
    )
    return task, (design_binding, spec_binding)


def _review_task(
    benchmark: ArtifactRef,
    design: SessionTask,
    game_spec: SessionTask,
    implementation: SessionTask,
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    task_id = "review-playability-and-visuals"
    sources = (
        (design, "design_bundle", "design_bundle"),
        (game_spec, "game_spec_bundle", "game_spec_bundle"),
        (implementation, "source_bundle", "source_bundle"),
    )
    bindings = tuple(
        ArtifactBinding(
            source_task_id=source.id,
            output_name=output_name,
            target_task_id=task_id,
            input_name=input_name,
        )
        for source, output_name, input_name in sources
    )
    task = _task(
        task_id=task_id,
        domain_function="pelican.verification.prototype_review.v1",
        instruction="""作为独立 QA/美术总监，审查第一版，不要修改 source 输入。
把源码复制到 review-bundle/.scratch，执行 Release build、--smoke-test，并用
--render-preview 生成 1440x900 画面。根据可用工具直接观察 PNG；若视觉工具不可用，
仍要结合像素尺寸、源码几何、层级和动画公式做结构审查，不得假装看过。完成证据
提取后删除 .scratch，最终 review-bundle 只保留下述三个文件。

只在 review-bundle 创建：
- preview.png：由原型自己的诊断模式生成，至少 1200x700；
- review.md：不少于 1200 个可见字符，按 composition、pelican anatomy、
  bicycle topology、rider contact、animation coupling、game feel、UI、accessibility、
  reliability、packaging 分项，先列 blocking 再列 polish；
- review.json：严格 JSON，含 verdict（PASS/NEEDS_POLISH/BLOCKED）、scores（上述
  维度 0-10）、blocking_issues、polish_issues、acceptance_results、commands。
  每个 issue 有 id、severity、evidence、required_fix；每项 acceptance result
  对应上游 acceptance test ID，并给 status/evidence。

必须特别寻找“增加装饰但没有修复根本构图”、腿脚没有踩踏板、车架/双轮错误、
动画把主体移出画面、UI 在小窗裁切、诊断模式不退出等鹈鹕基准常见失败。即使总体
通过，也至少提出三项有证据的微调建议供最终节点处理。完成后只申报 review_bundle。
""",
        inputs={
            "benchmark": benchmark,
            **{binding.input_name: binding for binding in bindings},
        },
        input_kinds={
            "benchmark": ArtifactKind.JSON,
            "design_bundle": ArtifactKind.DIRECTORY,
            "game_spec_bundle": ArtifactKind.DIRECTORY,
            "source_bundle": ArtifactKind.DIRECTORY,
        },
        output_name="review_bundle",
        output_path="review-bundle",
        description="independent build, screenshot and acceptance review",
        verifier="review_bundle",
        workspace_subdir="0300_review",
        stage=3000,
        topo_rank=2,
        order=1,
    )
    return task, bindings


def _delivery_task(
    benchmark: ArtifactRef,
    design: SessionTask,
    game_spec: SessionTask,
    implementation: SessionTask,
    review: SessionTask,
) -> tuple[SessionTask, tuple[ArtifactBinding, ...]]:
    task_id = "polish-and-package-exe"
    sources = (
        (design, "design_bundle", "design_bundle"),
        (game_spec, "game_spec_bundle", "game_spec_bundle"),
        (implementation, "source_bundle", "source_bundle"),
        (review, "review_bundle", "review_bundle"),
    )
    bindings = tuple(
        ArtifactBinding(
            source_task_id=source.id,
            output_name=output_name,
            target_task_id=task_id,
            input_name=input_name,
        )
        for source, output_name, input_name in sources
    )
    task = _task(
        task_id=task_id,
        domain_function="pelican.delivery.polish_package.v1",
        instruction="""完成最终修订、构建和证据封装。先逐项读取 review 的 blocking
与 polish issue；把 source_bundle 复制到 delivery-bundle/Source 后在副本上修复，
不得改上游。对所有改动重新运行 build、smoke 和 preview。

delivery-bundle 最终必须包含：
1. PelicanRide.exe：net8.0-windows/win-x64 自包含单文件 WPF WinExe；用户双击即玩，
   无需安装、联网或同目录 DLL/JSON/图片/音频。发布目录先放在
   delivery-bundle/.publish，再只保留这个 EXE 并删除 .publish。不得启用 trimming。
2. preview.png：最终 EXE 自己用 `--render-preview` 生成的 1440x900 画面。
3. README.md：简洁中文说明双击方式、玩法/按键、诊断参数、存档位置和卸载方式。
4. verification.json：严格 JSON，含 passed（仅全部 blocking check 通过时为 true）、
   status、checks（每项含 id/status/evidence）、product/version/platform、source_digest、
   exe_sha256/exe_bytes、build_command、build_exit_code、smoke_exit_code、
   preview_exit_code、preview_width/height、single_file、self_contained、
   runtime_network_required、loose_runtime_files、review_issues_resolved、
   acceptance_results、verified_at_utc。必须来自实际命令与文件，不能臆造。
5. Source/：最终可复现源码；不得含 bin、obj、.vs、publish、用户绝对路径或缓存。

发布后从一个与源码无关的 cwd 运行最终 EXE 的 --smoke-test 和 --render-preview，
再核对输出目录的运行必需文件确实只有 PelicanRide.exe。处理所有失败，直到全部
blocking acceptance 通过；只申报 delivery_bundle。
""",
        inputs={
            "benchmark": benchmark,
            **{binding.input_name: binding for binding in bindings},
        },
        input_kinds={
            "benchmark": ArtifactKind.JSON,
            "design_bundle": ArtifactKind.DIRECTORY,
            "game_spec_bundle": ArtifactKind.DIRECTORY,
            "source_bundle": ArtifactKind.DIRECTORY,
            "review_bundle": ArtifactKind.DIRECTORY,
        },
        output_name="delivery_bundle",
        output_path="delivery-bundle",
        description="verified source, preview and one-click PelicanRide.exe",
        verifier="delivery_bundle",
        workspace_subdir="0400_delivery",
        stage=4000,
        topo_rank=3,
        order=1,
    )
    return task, bindings


def build_pelican_plan(case_root: Path, workfolder: Path) -> WorkflowPlan:
    """Build the parallel-compose -> implement -> review -> delivery plan."""
    case_root = case_root.expanduser().resolve(strict=True)
    workfolder = workfolder.expanduser().resolve(strict=True)
    benchmark = _benchmark_ref(case_root)

    design = _design_task(benchmark)
    game_spec = _game_spec_task(benchmark)
    implementation, implementation_bindings = _implementation_task(
        benchmark,
        design,
        game_spec,
    )
    review, review_bindings = _review_task(
        benchmark,
        design,
        game_spec,
        implementation,
    )
    delivery, delivery_bindings = _delivery_task(
        benchmark,
        design,
        game_spec,
        implementation,
        review,
    )
    tasks = {
        task.id: task
        for task in (design, game_spec, implementation, review, delivery)
    }
    identity = hashlib.sha256(
        (
            benchmark.digest
            + str(workfolder).casefold()
            + "pelican-ride-workflow-v1"
        ).encode("utf-8")
    ).hexdigest()[:20]
    return WorkflowPlan(
        id=f"pelican-ride-{identity}",
        tasks=FrozenDict[SessionTask](tasks),
        bindings=(
            *implementation_bindings,
            *review_bindings,
            *delivery_bindings,
        ),
        policies=WorkflowPolicies(
            max_concurrency=2,
            failure_policy=FailurePolicy.SKIP_DEPENDENTS,
            fail_fast=False,
            max_subagent_depth=2,
            max_fan_out=12,
        ),
        outputs=FrozenDict[ArtifactBinding](
            {
                "delivery_bundle": ArtifactBinding(
                    source_task_id=delivery.id,
                    output_name="delivery_bundle",
                )
            }
        ),
    )


__all__ = ["build_pelican_plan"]
