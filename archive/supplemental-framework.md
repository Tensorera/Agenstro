# 已归档 — Agent Clef 概念框架（实现前增补稿）

> **Status: archived.** 本页是实现前的设计稿，保留用于追溯设计来源。
> 其中的包结构、阶段范围和待决事项可能已经过时。当前行为只以
> `docs/` 下的现行文档和源码为准。

## 1. 定位

这个框架可以被理解为一个 **effectful agent workflow runtime**：Python 负责组合、调度、验证和记录；OpenCode agent 负责执行含不确定性的领域函数。

目标不是让大模型本身变得确定，而是让以下内容尽量可复现、可检查：

- 相同的任务定义、输入 Artifact、Profile 和运行策略能够形成稳定的执行请求；
- 每次运行的输入、输出、副作用、模型与工具环境均可追踪；
- 不符合契约的模型输出不会直接成为可信结果；
- 复杂工作流可以先编译成明确的执行图，再由调度器运行；
- subagent 的创建、依赖、资源占用、失败传播和结果汇合由框架统一管理。

因此，“确定性”应当分成三层：

1. **定义确定性**：同一份 Python workflow 会编译出相同的规范化计划。
2. **执行可复现性**：固定输入、Profile、模型参数和工具环境后，运行具有可追踪的重试、缓存与审计语义。
3. **结果可验证性**：结果是否可接受由程序化契约判断，而不是只相信 agent 的自述。

大模型执行仍然可能不确定；框架提供的是受控不确定性，而不是承诺逐 token 相同。

## 2. 总体分层

建议分成五层，而不是让领域对象直接依赖 OpenCode 的具体 API：

```text
User Python Script
        |
        v
Domain API (Task / Function / Artifact / Plan)
        |
        v
Compiler (normalize / validate / bind / build DAG)
        |
        v
Runtime (schedule / execute / verify / retry / cache)
        |
        v
OpenCode Adapter (session / prompt / event / tool integration)
```

OpenCode Adapter 是防腐层。即使 OpenCode 的 session API、消息格式或启动方式变化，上层领域模型也不需要随之改变。

## 3. Python 与 agent 的信息传递协议

### 3.1 请求与响应必须有版本

协议建议采用可 JSON 序列化的 envelope，并显式包含：

```python
class AgentRequest:
    protocol_version: str
    run_id: str
    task_id: str
    attempt: int
    workspace: str
    prompts: list[Prompt]
    inputs: list[ArtifactRef]
    expected_outputs: list[ArtifactSpec]
    allowed_effects: EffectPolicy
    context: ContextRef | None


class AgentReport:
    protocol_version: str
    run_id: str
    task_id: str
    text: str
    artifacts: list[ArtifactClaim]
    state: RunState
    error: ErrorInfo | None
    context: ContextRef | None
```

请求中的 ID 由框架生成，agent 只能原样回传。`protocol_version` 用于渐进升级和拒绝不兼容响应。

### 3.2 模型自报与框架观测必须分离

agent 生成的 AST 本质上是不可信的 `AgentReport`，它只能表达“agent 声称发生了什么”。框架还要独立产生 `ExecutionTrace`：

- session 的真实开始和结束时间；
- agent/tool 事件流；
- 命令、退出码和截断后的 stdout/stderr；
- workspace 运行前后的文件快照或变更集；
- 实际创建、修改、移动、删除的路径；
- 验证器结果和失败原因。

最终 `SessionResult` 应由以下三者归并而成：

```text
AgentReport + ExecutionTrace + VerificationReport -> SessionResult
```

例如，agent 报告生成了 `report.md`，但文件不存在，此 Artifact 不能进入已验证输出。反过来，如果文件实际被创建但 agent 没有申报，框架将其记录为未申报变化，但不会据此替 OpenCode 作权限判定。

### 3.3 不要依赖自由文本中嵌入 JSON

优先级建议如下：

1. OpenCode 若支持结构化输出或可拦截的最终消息，直接使用；
2. 否则使用带唯一哨兵的末尾 envelope；
3. 解析失败时允许一次“仅修复格式”的受限重试；
4. 仍失败则返回 `PROTOCOL_ERROR`，不能猜测字段。

自由文本 `text` 与机器字段分开，避免自然语言里的花括号、代码块破坏解析。

### 3.4 Prompt 列表不是 chain of thought

`prompts: list[Prompt]` 可以表示分阶段指令、系统约束、任务说明、补充材料或修复提示，但不应将其定义为 chain of thought。框架不需要索取或存储模型的隐藏推理；需要的是可审计的结论、证据、工具事件和验证结果。

建议 Prompt 至少包含：

```python
class Prompt:
    role: Literal["policy", "instruction", "context", "repair"]
    content: str
    name: str | None
    priority: int
```

发送前按照确定的优先级和顺序规范化，生成 prompt digest。

## 4. 核心领域对象

### 4.1 Artifact

“路径 + 描述”适合作为最小界面，但内部建议补足身份、状态和来源：

```python
class ArtifactRef:
    uri: str
    description: str
    kind: Literal["file", "directory", "text", "json", "virtual"]
    digest: str | None
    media_type: str | None
    provenance: ArtifactProvenance | None


class ArtifactSpec:
    name: str
    description: str
    kind: str
    path: str | None
    required: bool = True
    constraints: list[ArtifactConstraint] = []
```

几个重要区分：

- `ArtifactRef` 是已经存在、可被消费的具体实例；
- `ArtifactSpec` 是期望产物的声明，不应伪装成已经存在的 Artifact；
- 文件身份最好使用内容摘要，而不是只使用可变路径；
- 删除也是 Artifact 事件，但不是一个“输出文件”，应表示为 `ArtifactChange`；
- text Artifact 可以存入内容寻址存储，再通过 URI 引用，避免所有中间文本都塞入 prompt。

路径进入 prompt 之前必须规范化，并验证位于允许的 workspace 内。不能只依靠字符串前缀判断路径归属。

### 4.2 Context

建议不要把 Context 定义成无法解释的整段模型上下文。更稳定的定义是：

```python
class ContextRef:
    session_id: str | None
    checkpoint_id: str | None
    summary_artifact: ArtifactRef | None
    message_range: tuple[int, int] | None
```

Context 是继续某次执行的引用或经过压缩的显式 Artifact。它应有大小预算、来源和版本，不能成为隐形输入；否则缓存键和复现记录都不完整。

### 4.3 SessionTask

```python
class SessionTask:
    id: str
    domain_function: str
    prompts: list[Prompt]
    inputs: dict[str, ArtifactRef | ArtifactBinding]
    outputs: dict[str, ArtifactSpec]
    contract: DomainContract
    context: ContextRef | None
    metadata: dict[str, JSONValue]
```

`ArtifactBinding` 用来表达“此输入来自上游任务的某个输出”，这样 workflow 在尚未执行时仍然可以被完整描述和验证。

### 4.4 SessionResult

```python
class SessionResult:
    run_id: str
    task: SessionTask
    attempt: int
    state: RunState
    outputs: dict[str, ArtifactRef]
    changes: list[ArtifactChange]
    text: str
    error: ErrorInfo | None
    verification: VerificationReport
    trace: ExecutionTraceRef
```

结果保存任务快照或任务定义摘要，避免任务对象后来被修改导致历史记录失真。

### 4.5 DomainContract：IOPQERV

原始的七元结构非常适合作为领域函数契约：

```text
F = <I, O, P, Q, E, R, V>
```

- `I`：具名输入槽位及其 Artifact 类型；
- `O`：具名输出规格；
- `P`：运行前可执行的纯验证规则；
- `Q`：运行后必须成立的条件；
- `E`：发送给 agent 的预期副作用集合；
- `R`：隔离、并发、重试和执行策略；
- `V`：验证器链。

建议进一步明确：

- `P` 失败时任务不启动，状态为 `REJECTED`；
- `Q` 是声明，`V` 是检查声明的实现；
- `E` 是任务意图和观测分类，不授予工具权限；实际副作用仍进入 trace；
- `R` 包含模型选择、温度/seed（若后端支持）、token/时间/费用预算、工作目录锁和最大尝试次数；
- `V` 可以包含确定性验证器和 agent 验证器，但后者不能替代文件存在性、schema、测试退出码等确定性检查。

验证器建议返回结构化结果，而不是简单布尔值：

```python
class VerificationReport:
    passed: bool
    checks: list[CheckResult]
    score: float | None
    evidence: list[ArtifactRef]
```

### 4.6 WorkflowPlan

概念上建议将 **SessionTask 作为节点，依赖关系作为边**。`SessionResult` 是节点的一次运行记录，而不是静态有向边。

```text
Static plan:   Task A --output binding--> Task B
Runtime view:  Result(A, attempt 1) ------> Run(B, attempt 1)
```

这样有几个好处：

- 编译期不需要虚构尚未产生的 SessionResult；
- 同一节点的重试会产生多个 Result，不会改变计划拓扑；
- 边可以明确描述输出到输入的具名映射；
- 条件分支、fan-out、fan-in 和动态展开更容易表达。

如果希望保留“Result 是边”的直觉，可以把它作为运行期派生图，而不是 `WorkflowPlan` 的基础存储模型。

计划至少需要：

```python
class WorkflowPlan:
    id: str
    tasks: dict[str, SessionTask]
    bindings: list[ArtifactBinding]
    policies: WorkflowPolicies
    outputs: dict[str, ArtifactBinding]
```

第一版应只支持 DAG。循环、递归和 agent 动态生成节点会显著增加终止性、预算和可复现性问题，可留给后续版本。

## 5. 运行状态与错误

`run_state` 建议使用有限状态机，而不是任意字符串：

```text
PENDING -> READY -> RUNNING -> VERIFYING -> SUCCEEDED
                    |              |
                    v              v
                 BLOCKED         FAILED
                    |
                    v
              WAITING_INPUT

任意未结束状态 -> CANCELLED
OpenCode/provider 报告 timeout -> TIMED_OUT
前置条件失败    -> REJECTED
```

`BLOCKED` 表示 agent 识别到障碍但当前运行无法自行解决；它与框架异常 `FAILED` 不同。是否重试由错误类别和策略共同决定。

```python
class ErrorInfo:
    code: str
    category: Literal[
        "protocol",
        "precondition",
        "agent",
        "tool",
        "resource",
        "verification",
        "permission",
        "dependency",
        "internal",
    ]
    message: str
    retryable: bool
    details: dict[str, JSONValue]
    cause: ErrorInfo | None
```

错误中不要存放凭据或无限量 stdout/stderr；大内容写入 trace Artifact。

## 6. 基本领域函数

建议公开 Python 风格 API，同时保留原始概念名作为文档术语：

```python
def domain_run(
    task: SessionTask,
    *,
    profile: Profile,
) -> SessionResult: ...


def execute_plan(
    plan: WorkflowPlan,
    *,
    profile: Profile,
) -> WorkflowResult: ...
```

`ExcutePlan` 建议修正为 `execute_plan`。计划执行返回 `WorkflowResult`，而不只是 `list[SessionResult]`，因为还需要表达：

- 计划整体状态；
- 每个 task 的零次、一次或多次 attempt；
- 最终命名输出；
- 未运行或被上游失败跳过的节点；
- 总资源消耗与完整 trace。

可以再提供这些基础能力：

```python
compile_workflow(definition, profile) -> WorkflowPlan
validate_plan(plan, profile) -> PlanValidationReport
verify_result(result, verifier) -> VerificationReport
resume_run(run_id, profile) -> WorkflowResult
cancel_run(run_id, profile) -> None
```

`create_plan(session_task)` 暂时不实现是合理的。第一阶段先允许用户用普通 Python 显式构图；后续再加入由 agent 生成候选计划、由编译器进行静态检查的能力。

## 7. 编译过程

所谓“复杂 agent workflow 的编译”，可以落实为以下确定性阶段：

1. **Capture**：Python DSL 捕获 task、binding、policy。
2. **Normalize**：补齐默认值，排序 prompts，规范化路径和 ID。
3. **Type check**：检查上游 `ArtifactSpec` 是否能绑定到下游输入。
4. **Contract check**：检查前后置条件、effect 和 verifier 是否完整。
5. **Graph check**：检测环、缺失依赖、不可达节点和重名输出。
6. **Runtime bind**：将 Profile 中的 adapter、模型、路径、存储和并发配置注入计划。
7. **Digest**：对规范化计划生成摘要，作为缓存和审计身份。

编译产物应该可序列化，以便 dry-run、审阅、保存和比较。Python 闭包本身通常不可稳定序列化，因此领域函数、验证器和条件最好使用注册名加参数，而不是任意 lambda。

## 8. 调度与 subagent 协调

第一版调度器可以采用 ready-queue：

1. 入度为零且前置条件满足的节点进入 `READY`；
2. 根据 Profile 中的全局并发数、模型配额和 workspace 锁选择任务；
3. 执行完成并验证成功后，发布其输出 Artifact；
4. 绑定完成的下游节点重新计算就绪状态；
5. 上游失败时根据 edge/task policy 决定跳过、降级、重试或继续；
6. 所有终端节点结束后汇总 `WorkflowResult`。

subagent 不应被看成特殊线程，而是另一类受相同契约约束的 DomainRun。建议增加：

- 父子 run ID 和调用深度；
- 最大 fan-out、最大深度、总 token/费用预算；
- 每个 workspace 的读写锁；
- 取消信号向子运行传播；
- 子任务输出必须经过 Artifact/Verifier 边界后才能汇入父任务；
- 禁止 subagent 绕过 scheduler 自行创建不可见任务。

多个任务若会写同一 workspace，默认串行；只有声明为只读或写入互不相交的路径集合时才允许并行。

## 9. 副作用与权限模型

Clef 不再维护 OpenCode permission 的影子副本。Profile 只把明确的工作目录
传给 OpenCode，并使用该目录完成 Artifact 路径解析和验收；shell、network、
edit、delete、move、subagent、external directory 等权限全部由 OpenCode 的
`permission` 配置决定。

当前默认 `adapter.auto_approve=true`，即调用 `opencode run --auto`。它会自动
批准 OpenCode 中的 `ask`，但不会覆盖显式 `deny`。如果需要更细粒度的命令规则，
应直接在全局或项目级 `opencode.json` 中配置，例如：

```json
{
  "$schema": "https://opencode.ai/config.json",
  "permission": {
    "*": "allow",
    "bash": {
      "*": "allow",
      "git push *": "ask"
    }
  }
}
```

Clef 继续记录 workspace diff 和 tool events，但这些记录只用于可观测性、
错误修复与 Artifact 验证，不构成第二套授权系统。若需要处理不可信仓库或秘密
数据，仍应使用容器、受限用户、文件系统挂载和网络策略等真实隔离。

## 10. Profile 与依赖注入

按用户区分 Profile 是合理的。建议的 Python 使用方式：

```python
from clef_sdk import domain_run
from clef_sdk.profiles import load_profile

profile = load_profile("default")
result = domain_run(task, profile=profile)
```

默认搜索路径可以是：

```text
<user-config-dir>/clef_sdk/profiles/<name>.toml
```

Profile 应是数据，而不是带任意副作用的 Python 模块；`load_profile()` 将其解析为不可变对象并完成校验。秘密信息只保存环境变量名或 secret provider 引用，不直接写入 TOML。

Profile 可以包含：

- OpenCode adapter 与可执行文件位置；
- 模型和模型参数；
- Artifact workspace 与额外输入读取路径；
- 并发、重试和 session 恢复策略；token 与费用只作为观测数据；
- Artifact store、trace store 和 cache 位置；
- 默认 verifier 与错误策略。

每次运行都显式注入 Profile，不使用隐藏的全局单例。运行记录中保存脱敏后的 Profile 快照与摘要，以确保能解释一次执行使用了什么策略。

## 11. 缓存、重试与幂等性

缓存键至少应覆盖：

```text
domain function version
+ normalized task
+ input artifact digests
+ prompt digest
+ profile digest
+ model/tool/runtime identity
+ protocol version
```

只有声明为可缓存、且副作用可以重放或已经物化为 Artifact 的任务才能命中缓存。

重试不是原地覆盖。每次尝试都生成新的 attempt 和 trace。对会修改 workspace 的任务，重试前必须选择明确策略：

- 从干净快照恢复；
- 在当前状态继续；
- 创建新 workspace；
- 禁止自动重试。

框架不能假设 agent 操作天然幂等。

## 12. 建议的最小 Python 包边界

```text
clef_sdk/
  __init__.py
  model/
    artifact.py
    prompt.py
    task.py
    result.py
    contract.py
    workflow.py
  protocol/
    envelope.py
    codec.py
  adapters/
    opencode.py
  compiler/
    compile.py
    validate.py
  runtime/
    runner.py
    scheduler.py
    state.py
  verification/
    base.py
    builtin.py
  profiles/
    loader.py
    schema.py
  storage/
    artifacts.py
    traces.py
    cache.py
```

目标版本为 Python 3.12；本机当前版本为 Python 3.12.10。

## 13. 第一阶段实现范围

建议用一个很窄的 MVP 验证核心闭环：

1. dataclass/Pydantic 风格的核心对象和 JSON 协议；
2. 一个 OpenCode session adapter；
3. 单任务 `domain_run()`；
4. workspace 变更观测；
5. 文件存在性、digest、JSON Schema、命令退出码四类内置 verifier；
6. 静态 DAG 的 `execute_plan()`，先串行，随后加入有限并行；
7. JSONL trace 和本地内容寻址 Artifact store；
8. TOML Profile 加显式依赖注入；
9. 一个 fan-out/fan-in 示例 workflow；
10. 用 fake adapter 做完全确定性的单元测试，用真实 OpenCode 做集成测试。

暂缓动态规划、循环工作流、分布式调度和强安全沙箱。先证明：

```text
定义任务 -> 编译计划 -> 执行 session -> 观测副作用
        -> 解析报告 -> 验证 Artifact -> 调度下游 -> 留下可复现记录
```

## 14. 需要尽早定下来的设计决策

后续实现前，建议优先形成 ADR：

1. OpenCode adapter 采用 CLI、服务 API，还是两者兼容；
2. 结构化最终报告如何从 session 中可靠提取；
3. workspace 变更通过版本控制 diff、文件快照还是工具事件获得；
4. Artifact store 是否复制文件，还是只记录原路径与 digest；
5. 默认失败传播、重试和补偿策略；
6. agent verifier 在验证链中的权重与权限；
7. Context 的保留、压缩、脱敏和过期规则；
8. 动态 subagent 是否只允许通过 scheduler 创建。

其中最重要的原则是：**模型负责提出和执行，框架负责观察、约束、验证与记账。**
