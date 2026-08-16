# `clef_sdk.model` reference

本卷收录公共模型与 JSON API。所有符号均可从 `clef_sdk.model` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`ArtifactBinding`](#artifactbinding) | Entity | dataclass | task 输出到 task 输入的绑定 |
| [`ArtifactChange`](#artifactchange) | Entity | dataclass | TaskRun 文件变化记录 |
| [`ArtifactChangeKind`](#artifactchangekind) | Entity | enum | TaskRun 文件变化类型 |
| [`ArtifactClaim`](#artifactclaim) | Entity | dataclass | AgentReport 中的输出声明 |
| [`ArtifactConstraint`](#artifactconstraint) | Entity | dataclass | ArtifactSpec 的确定性约束 |
| [`ArtifactConstraintKind`](#artifactconstraintkind) | Entity | enum | Artifact 约束类型 |
| [`ArtifactKind`](#artifactkind) | Entity | enum | Artifact 数据类型 |
| [`ArtifactMutation`](#artifactmutation) | Entity | dataclass | artifact tree 的一个净变化 |
| [`ArtifactMutationKind`](#artifactmutationkind) | Entity | enum | create、update 或 delete |
| [`ArtifactProvenance`](#artifactprovenance) | Entity | dataclass | Artifact 来源信息 |
| [`ArtifactRef`](#artifactref) | Entity | dataclass | 已存在 Artifact 引用 |
| [`ArtifactSpec`](#artifactspec) | Entity | dataclass | task 预期输出定义 |
| [`canonical_json_dumps`](#canonical_json_dumps) | API | public API | 将 JSON 值编码为规范字符串 |
| [`canonical_json_loads`](#canonical_json_loads) | API | public API | 将严格 JSON 字符串解码为 Python 值 |
| [`CheckResult`](#checkresult) | Entity | dataclass | 单个 verifier 结果 |
| [`CheckStatus`](#checkstatus) | Entity | enum | verifier 状态 |
| [`ContextRef`](#contextref) | Entity | dataclass | 显式上下文引用 |
| [`DomainContract`](#domaincontract) | Entity | dataclass | task 输入输出和运行约束 |
| [`EffectKind`](#effectkind) | Entity | enum | 副作用类型 |
| [`EffectPolicy`](#effectpolicy) | Entity | dataclass | 发给 agent 的 effect 意图 |
| [`EffectRule`](#effectrule) | Entity | dataclass | 一条副作用路径规则 |
| [`Effort`](#effort) | Entity | enum | task 的逻辑模型路由档位 |
| [`ErrorCategory`](#errorcategory) | Entity | enum | 运行错误分类 |
| [`ErrorInfo`](#errorinfo) | Entity | dataclass | 结构化错误 |
| [`ExecutionTraceRef`](#executiontraceref) | Entity | dataclass | trace 文件引用 |
| [`FailurePolicy`](#failurepolicy) | Entity | enum | 已序列化、当前 scheduler 尚未消费的失败策略词汇 |
| [`freeze_json`](#freeze_json) | API | public API | 将 JSON 容器转换为 immutable 值 |
| [`FrozenDict`](#frozendict) | Entity | mapping | immutable mapping |
| [`JsonModel`](#jsonmodel) | Entity | base class | 严格 JSON entity 基类 |
| [`ModelDecodeError`](#modeldecodeerror) | Entity | exception | model 解码错误 |
| [`ModelError`](#modelerror) | Entity | exception | model 错误基类 |
| [`ModelValidationError`](#modelvalidationerror) | Entity | exception | model 字段校验错误 |
| [`Prompt`](#prompt) | Entity | dataclass | 一条 prompt 消息 |
| [`PromptRole`](#promptrole) | Entity | enum | prompt 角色 |
| [`ResourcePolicy`](#resourcepolicy) | Entity | dataclass | task attempt、cache 和 retry 配置 |
| [`ResourceUsage`](#resourceusage) | Entity | dataclass | workflow 聚合资源用量 |
| [`RetryWorkspaceStrategy`](#retryworkspacestrategy) | Entity | enum | retry workspace 策略 |
| [`RuleSpec`](#rulespec) | Entity | dataclass | precondition 或 postcondition 定义 |
| [`RunProgressEvent`](#runprogressevent) | Entity | dataclass | 一条 typed workflow 运行摘要事件 |
| [`RunState`](#runstate) | Entity | enum | SessionResult 状态 |
| [`SessionResult`](#sessionresult) | Entity | dataclass | TaskRun attempt 结果 |
| [`SessionTask`](#sessiontask) | Entity | dataclass | agent 任务定义 |
| [`TASK_RESULT_SCHEMA`](#taskresultenvelope) | Entity | constant | TaskResultEnvelope v1 schema tag |
| [`TaskResultEnvelope`](#taskresultenvelope) | Entity | dataclass | domain JSON 与实际 artifact 变化 |
| [`TaskInput`](#taskinput) | Entity | type alias | task 输入联合类型 |
| [`TaskProgressState`](#taskprogressstate) | Entity | enum | task 的实时调度阶段 |
| [`TaskRunSummary`](#taskrunsummary) | Entity | dataclass | 一个 task 的当前运行摘要 |
| [`thaw_json`](#thaw_json) | API | public API | 将 immutable JSON 值转换为普通容器 |
| [`parse_task_result`](#parse_task_result) | API | public API | 解析 envelope 或包装旧任意 JSON |
| [`VerificationReport`](#verificationreport) | Entity | dataclass | verifier chain 聚合结果 |
| [`VerifierSpec`](#verifierspec) | Entity | dataclass | verifier 调用定义 |
| [`WorkflowPlan`](#workflowplan) | Entity | dataclass | 静态 DAG 定义 |
| [`WorkflowPolicies`](#workflowpolicies) | Entity | dataclass | workflow 调度策略 |
| [`WorkflowResult`](#workflowresult) | Entity | dataclass | workflow 执行结果 |
| [`WorkflowRunSummary`](#workflowrunsummary) | Entity | dataclass | workflow 的实时或终态摘要 |
| [`WorkflowState`](#workflowstate) | Entity | enum | workflow 状态 |

## ArtifactBinding

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactBinding`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactBinding
```

Namespace：`clef_sdk.model`

输入字段：`source_task_id`、`output_name`、`target_task_id`、`input_name`。

输出：一个上游输出引用。`target_task_id` 和 `input_name` 同时存在时形成显式 DAG
边。

## ArtifactChange

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactChange`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactChange
```

Namespace：`clef_sdk.model`

输入字段：`kind`、`path`、`previous_path`、`before_digest`、`after_digest`、
`declared`。

输出：SessionResult 中的一条文件变化记录。

## ArtifactChangeKind

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactChangeKind`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactChangeKind
```

Namespace：`clef_sdk.model`

值：

```text
CREATED=created
DELETED=deleted
MODIFIED=modified
MOVED=moved
```

输出：ArtifactChange 的变化分类。

## ArtifactClaim

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactClaim`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactClaim
```

Namespace：`clef_sdk.model`

输入字段：`name`、`uri`、`description`、`kind`、`digest`、`media_type`。

输出：AgentReport 中的一条 Artifact 声明。

## ArtifactConstraint

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactConstraint`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactConstraint
```

Namespace：`clef_sdk.model`

输入字段：

- `kind: ArtifactConstraintKind`；
- `parameters: FrozenDict[Any]`。

输出：ArtifactSpec 的确定性验证约束。

`kind=JSON_SCHEMA` 时，`parameters` 必须包含 `schema`。其值可以是 Schema
object 或 boolean schema，并统一按 JSON Schema Draft 2020-12 解释。

## ArtifactConstraintKind

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactConstraintKind`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactConstraintKind
```

Namespace：`clef_sdk.model`

值：

```text
DIGEST=digest
EXISTS=exists
JSON_SCHEMA=json_schema
MAX_BYTES=max_bytes
MEDIA_TYPE=media_type
MIN_BYTES=min_bytes
```

输出：ArtifactConstraint 类型。

## ArtifactKind

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactKind`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactKind
```

Namespace：`clef_sdk.model`

值：

```text
DIRECTORY=directory
FILE=file
JSON=json
TEXT=text
VIRTUAL=virtual
```

输出：ArtifactRef、ArtifactSpec 和 DomainContract 使用的数据类型。

## ArtifactMutation

**Canonical FQN（规范完全限定名）**：
`clef_sdk.model.ArtifactMutation`

```python
from clef_sdk.model import (
    ArtifactKind,
    ArtifactMutation,
    ArtifactMutationKind,
)

change = ArtifactMutation(
    operation=ArtifactMutationKind.UPDATE,
    path="src/app.py",
    kind=ArtifactKind.FILE,
    description="Validated implementation",
)
```

字段：`operation`、`path`、`kind`、`description`。`path` 必须是规范项目相对
路径；绝对路径、空路径、`.` 和 `..` 片段被拒绝。这个新模型只接受物理
`file` 和 `directory` kind，不会缩窄旧 `ArtifactSpec` 的 semantic kinds。

## ArtifactMutationKind

**Canonical FQN（规范完全限定名）**：
`clef_sdk.model.ArtifactMutationKind`

```text
CREATE=create
UPDATE=update
DELETE=delete
```

Delete 仍携带 kind，从而让 history 保留 tombstone 的物理类型。

## ArtifactProvenance

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactProvenance`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactProvenance
```

Namespace：`clef_sdk.model`

输入字段：`producer_task_id`、`run_id`、`source_uri`、`metadata`。

输出：ArtifactRef 的来源记录。

## ArtifactRef

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactRef`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactRef
```

Namespace：`clef_sdk.model`

输入字段：`uri`、`description`、`kind`、`digest`、`media_type`、`provenance`。

输出：已存在 Artifact 的 immutable 引用。

## ArtifactSpec

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ArtifactSpec`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ArtifactSpec
```

Namespace：`clef_sdk.model`

输入字段：`name`、`description`、`kind`、`path`、`required`、`constraints`。

输出：SessionTask 的预期输出定义。

## canonical_json_dumps

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.canonical_json_dumps`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import canonical_json_dumps
```

```python
canonical_json_dumps(value: Any) -> str
```

输入：JSON 兼容值或框架 immutable JSON 值。

输出：key 排序、紧凑分隔符和稳定 Unicode 处理后的 JSON 字符串。

## canonical_json_loads

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.canonical_json_loads`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import canonical_json_loads
```

```python
canonical_json_loads(payload: str) -> Any
```

输入：JSON 字符串。

输出：严格解码后的 Python 值。object key 重复、NaN 和 Infinity 触发
`ModelDecodeError`。

## CheckResult

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.CheckResult`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import CheckResult
```

Namespace：`clef_sdk.model`

输入字段：`name`、`status`、`message`、`required`、`score`、`evidence`、
`details`。

输出：一个 verifier 的 immutable 结果。

## CheckStatus

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.CheckStatus`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import CheckStatus
```

Namespace：`clef_sdk.model`

值：

```text
ERROR=error
FAILED=failed
PASSED=passed
SKIPPED=skipped
```

输出：CheckResult 状态。

## ContextRef

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ContextRef`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ContextRef
```

Namespace：`clef_sdk.model`

输入字段：`session_id`、`checkpoint_id`、`summary_artifact`、`message_range`。

输出：显式上下文引用。跨 TaskRun 信息通过 `summary_artifact` 进入计划。

## DomainContract

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.DomainContract`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import DomainContract
```

Namespace：`clef_sdk.model`

输入字段：

- `inputs: FrozenDict[ArtifactKind]`；
- `outputs: FrozenDict[ArtifactKind]`；
- `preconditions: tuple[RuleSpec, ...]`；
- `postconditions: tuple[RuleSpec, ...]`；
- `effects: EffectPolicy`；
- `resources: ResourcePolicy`；
- `verifiers: tuple[VerifierSpec, ...]`。

输出：SessionTask 的执行 Contract。`effects` 表达任务意图，不是 OpenCode
permission 的替代品。

## EffectKind

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.EffectKind`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import EffectKind
```

Namespace：`clef_sdk.model`

值：

```text
CREATE=create
DELETE=delete
MODIFY=modify
MOVE=move
NETWORK=network
READ=read
SHELL=shell
```

输出：EffectRule 的副作用类型。

## EffectPolicy

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.EffectPolicy`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import EffectPolicy
```

Namespace：`clef_sdk.model`

输入字段：

- `allowed: tuple[EffectRule, ...]`。

输出：发送给 OpenCode 的 task effect 意图。该对象不是权限边界；实际工具授权
由 OpenCode 原生 `permission` 配置决定。

## EffectRule

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.EffectRule`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import EffectRule
```

Namespace：`clef_sdk.model`

输入字段：

- `kind: EffectKind`；
- `path_glob: str | None`。

输出：一条副作用类型与路径规则。

## ErrorCategory

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ErrorCategory`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ErrorCategory
```

Namespace：`clef_sdk.model`

值：

```text
AGENT=agent
DEPENDENCY=dependency
INTERNAL=internal
PERMISSION=permission
PRECONDITION=precondition
PROTOCOL=protocol
RESOURCE=resource
TOOL=tool
VERIFICATION=verification
```

输出：ErrorInfo 的分类。

## ErrorInfo

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ErrorInfo`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ErrorInfo
```

Namespace：`clef_sdk.model`

输入字段：`code`、`category`、`message`、`retryable`、`details`、`cause`。

输出：SessionResult 或 WorkflowResult 的结构化错误。

## ExecutionTraceRef

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ExecutionTraceRef`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ExecutionTraceRef
```

Namespace：`clef_sdk.model`

输入字段：`uri`、`digest`。

输出：trace 文件的引用。

## FailurePolicy

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.FailurePolicy`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import FailurePolicy
```

Namespace：`clef_sdk.model`

值：

```text
CONTINUE=continue
FAIL_FAST=fail_fast
SKIP_DEPENDENTS=skip_dependents
```

输出：WorkflowPolicies 中序列化的失败策略词汇。v0.1 scheduler 尚未读取
`failure_policy`；当前行为由 `WorkflowPolicies.fail_fast`、
`RuntimeConfig.fail_fast` 和固定的 dependent-skip 规则决定。

## freeze_json

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.freeze_json`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import freeze_json
```

```python
freeze_json(
    value: Any,
    *,
    field: str = "value",
) -> Any
```

输入：JSON 兼容值和字段名称。

输出：dict 转换为 FrozenDict，list 转换为 tuple，标量保持 JSON 标量类型。

## FrozenDict

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.FrozenDict`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import FrozenDict
```

Namespace：`clef_sdk.model`

构造输入：

```python
FrozenDict(
    source: Mapping[str, T] | Iterable[tuple[str, T]] | None = None,
)
```

输出：immutable mapping。公开方法为 `get()`、`items()`、`keys()`、`to_dict()`
和 `values()`。

## JsonModel

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.JsonModel`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import JsonModel
```

Namespace：`clef_sdk.model`

输入：子类字段 mapping 或 JSON 字符串。

输出：严格 JSON entity。

公开方法：

```python
from_dict(data: Mapping[str, Any]) -> Self
from_json(payload: str) -> Self
to_dict() -> dict[str, Any]
to_json() -> str
```

## ModelDecodeError

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ModelDecodeError`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ModelDecodeError
```

Namespace：`clef_sdk.model`

输入：异常消息。

输出：严格 JSON 解码产生的 `ModelError`。

## ModelError

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ModelError`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ModelError
```

Namespace：`clef_sdk.model`

输入：异常消息。

输出：model 错误基类实例。

## ModelValidationError

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ModelValidationError`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ModelValidationError
```

Namespace：`clef_sdk.model`

输入：异常消息。

输出：entity 字段校验产生的 `ModelError`。

## Prompt

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.Prompt`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import Prompt
```

Namespace：`clef_sdk.model`

输入字段：

- `role: PromptRole`；
- `content: str`；
- `name: str | None`；
- `priority: int`。

输出：SessionTask prompt stack 中的一条 immutable 消息。

## PromptRole

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.PromptRole`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import PromptRole
```

Namespace：`clef_sdk.model`

值：

```text
CONTEXT=context
INSTRUCTION=instruction
POLICY=policy
REPAIR=repair
```

输出：Prompt 角色。

## ResourcePolicy

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ResourcePolicy`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ResourcePolicy
```

Namespace：`clef_sdk.model`

输入字段：

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `max_attempts` | `int` | `1` | attempt 上限 |
| `retry_workspace_strategy` | `RetryWorkspaceStrategy` | `FORBID` | retry workspace 策略 |
| `cacheable` | `bool` | `False` | 调用方 cache eligibility 声明 |
| `concurrency_key` | `str \| None` | `None` | 并发资源键 |

输出：DomainContract 的 attempt、workspace retry、cache 和并发策略。该实体不再
声明 session timeout 或 token/cost ceiling。

runtime 使用 `max_attempts`、`retry_workspace_strategy` 和 `concurrency_key`；
`cacheable` 不会触发自动 cache lookup/store，调用方仍须显式使用
ConservativeCache。

## ResourceUsage

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.ResourceUsage`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import ResourceUsage
```

Namespace：`clef_sdk.model`

输入字段：`prompt_tokens`、`completion_tokens`、`attempts`、`wall_time_seconds`、
`cost_usd`。

输出：WorkflowResult 的聚合观测数据，不是 TaskRun enforcement policy。

## RetryWorkspaceStrategy

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.RetryWorkspaceStrategy`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import RetryWorkspaceStrategy
```

Namespace：`clef_sdk.model`

值：

```text
CONTINUE=continue
FORBID=forbid
NEW=new
RESTORE=restore
```

输出：ResourcePolicy 的 retry workspace 策略。

## RuleSpec

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.RuleSpec`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import RuleSpec
```

Namespace：`clef_sdk.model`

输入字段：

- `name: str`；
- `parameters: FrozenDict[Any]`。

输出：precondition 或 postcondition 的调用定义。

## RunProgressEvent

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.RunProgressEvent`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import RunProgressEvent
```

Namespace：`clef_sdk.model`

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `schema_version` | `str` | typed progress schema；当前默认 `1.0` |
| `sequence` | `int` | workflow 事件总序列 |
| `timestamp` | `str` | UTC 事件时间 |
| `elapsed_seconds` | `float` | workflow 开始后的单调耗时 |
| `run_id`, `plan_id` | `str` | 运行与计划 identity |
| `event` | `str` | typed progress 事件名称 |
| `scope` | `str` | `plan`、`workflow`、`task`、`attempt`、`agent`、`verification` 或 `publication` |
| `task_id` | `str \| None` | task scope identity |
| `attempt` | `int \| None` | attempt identity |
| `summary` | `WorkflowRunSummary` | 此事件时刻的 immutable 快照 |
| `data` | `FrozenDict[Any]` | 事件专属的严格 JSON 字段；长度随事件和计划规模变化 |

输出：传给 `ProgressObserver` 的一条 typed 运行摘要事件。`to_dict()` 与
`from_dict()` 提供严格 JSON round trip。`summary` 是稳定、聚合的状态面；
`data` 可包含路径、错误详情或计划级列表，不承诺固定长度，也不应由 UI 整块
渲染。需要安全、紧凑文本时使用 `format_progress_event()`。

## Effort

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.Effort`

```python
from clef_sdk import Effort
```

值：`xhigh`、`high`、`medium`、`low`。这些值只是 Clef Profile 的逻辑
路由名，不是 OpenCode 或模型 provider 的原生 reasoning effort/variant。

## RunState

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.RunState`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import RunState
```

Namespace：`clef_sdk.model`

值：

```text
BLOCKED=BLOCKED
CANCELLED=CANCELLED
FAILED=FAILED
PENDING=PENDING
READY=READY
REJECTED=REJECTED
RUNNING=RUNNING
SUCCEEDED=SUCCEEDED
TIMED_OUT=TIMED_OUT
VERIFYING=VERIFYING
WAITING_INPUT=WAITING_INPUT
```

输出：SessionResult 和 AgentReport 的状态。

## SessionResult

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.SessionResult`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import SessionResult
```

Namespace：`clef_sdk.model`

输入字段：`run_id`、`task`、`attempt`、`state`、`verification`、`trace`、
`outputs`、`changes`、`text`、`error`。

输出：一个 TaskRun attempt 的完整结果。

## SessionTask

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.SessionTask`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import SessionTask
```

Namespace：`clef_sdk.model`

输入字段：

- `id: str`；
- `domain_function: str`；
- `prompts: tuple[Prompt, ...]`；
- `inputs: FrozenDict[TaskInput]`；
- `outputs: FrozenDict[ArtifactSpec]`；
- `contract: DomainContract`；
- `context: ContextRef | None`；
- `metadata: FrozenDict[Any]`；
- `artifact_changes: tuple[ArtifactMutation, ...]`；
- `effort: Effort | None`。

输出：runtime 的最小任务定义。`artifact_changes` 声明 task 预期产生的
create/update/delete 净变化；同一路径在一个 task 中最多出现一次。省略
`effort` 时保持单一默认模型行为且序列化中不写入该字段；显式 effort 必须在
Profile 的 `adapter.models` 中有对应路由。

## TaskResultEnvelope

**Canonical FQN（规范完全限定名）**：
`clef_sdk.model.TaskResultEnvelope`

```python
from clef_sdk.model import TaskResultEnvelope
```

字段：

- `schema = "clef_sdk.task-result/v1"`；
- `result`：任意严格 JSON domain value；
- `artifacts: tuple[ArtifactMutation, ...]`：实际接受的变化；
- `summary: str`。

Envelope 不替换 domain schema。调用方可继续对解包后的 `result` 使用任意已有
JSON Schema。

## parse_task_result

```python
from clef_sdk.model import parse_task_result

envelope = parse_task_result(payload)
domain_value = envelope.result
changes = envelope.artifacts
```

输入：严格 JSON 字符串。带
`"schema": "clef_sdk.task-result/v1"` 的对象按 envelope 严格解析；
其他 object、array、scalar 或 null 都按旧 domain result 自动包装，
`artifacts=()`。重复 object key、NaN 和 Infinity 仍被拒绝。

## TaskInput

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.TaskInput`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import TaskInput
```

Namespace：`clef_sdk.model`

定义：

```python
TaskInput = ArtifactRef | ArtifactBinding
```

输入：已存在 Artifact 或计划内上游绑定。

输出：SessionTask.inputs 的值类型。

## TaskProgressState

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.TaskProgressState`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import TaskProgressState
```

Namespace：`clef_sdk.model`

值：

```text
PENDING=PENDING
SCHEDULED=SCHEDULED
RUNNING=RUNNING
VERIFYING=VERIFYING
REPAIRING=REPAIRING
PUBLISHING=PUBLISHING
SUCCEEDED=SUCCEEDED
FAILED=FAILED
SKIPPED=SKIPPED
```

输出：`TaskRunSummary.state` 使用的实时 scheduler 阶段。

## TaskRunSummary

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.TaskRunSummary`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import TaskRunSummary
```

Namespace：`clef_sdk.model`

输入字段：`task_id`、`state`、`attempt`、`session_number`、`turn_kind`、
`repair_turns`、`verification_passes`、`verification_failures`、
`published_outputs`、`elapsed_seconds`。

输出：一个 task 在某个 progress sequence 上的 immutable、有界聚合摘要。
`to_dict()` 与 `from_dict()` 提供严格 JSON round trip。

## thaw_json

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.thaw_json`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import thaw_json
```

```python
thaw_json(value: Any) -> Any
```

输入：框架 immutable JSON 值。

输出：普通 dict、list 和 JSON 标量。

## VerificationReport

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.VerificationReport`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import VerificationReport
```

Namespace：`clef_sdk.model`

输入字段：`passed`、`checks`、`score`、`evidence`。

输出：一个 verifier chain 的聚合结果。

## VerifierSpec

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.VerifierSpec`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import VerifierSpec
```

Namespace：`clef_sdk.model`

输入字段：

- `name: str`；
- `parameters: FrozenDict[Any]`；
- `required: bool`。

输出：DomainContract 中的一次 verifier 调用定义。

## WorkflowPlan

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.WorkflowPlan`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import WorkflowPlan
```

Namespace：`clef_sdk.model`

输入字段：

- `id: str`；
- `tasks: FrozenDict[SessionTask]`；
- `bindings: tuple[ArtifactBinding, ...]`；
- `policies: WorkflowPolicies`；
- `outputs: FrozenDict[ArtifactBinding]`。

输出：一个静态 DAG。

## WorkflowPolicies

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.WorkflowPolicies`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import WorkflowPolicies
```

Namespace：`clef_sdk.model`

输入字段：

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `max_concurrency` | `int` | `1` | workflow 并发上限 |
| `failure_policy` | `FailurePolicy` | `SKIP_DEPENDENTS` | 保留字段；当前 scheduler 不读取 |
| `fail_fast` | `bool` | `True` | 快速结束配置 |
| `max_subagent_depth` | `int` | `1` | 保留字段；当前不执行 depth enforcement |
| `max_fan_out` | `int` | `32` | fan-out 上限 |

输出：WorkflowPlan 的调度策略。

当前执行路径使用 `max_concurrency` 和 `fail_fast`；`max_fan_out` 在编译期检查。
`failure_policy` 与 `max_subagent_depth` 会被校验和序列化，但尚未驱动 runtime
分支。

## WorkflowResult

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.WorkflowResult`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import WorkflowResult
```

Namespace：`clef_sdk.model`

输入字段：`run_id`、`plan_id`、`state`、`trace`、`task_results`、`outputs`、
`skipped_tasks`、`usage`、`error`、`summary`。

输出：一个 WorkflowPlan 的完整执行结果。SDK scheduler 返回的结果在
`summary` 中包含 terminal `WorkflowRunSummary`；从旧 payload 反序列化时该字段
可以为 `None`，且 `summary=None` 时 `to_dict()` 不写入该键。

## WorkflowRunSummary

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.WorkflowRunSummary`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import WorkflowRunSummary
```

Namespace：`clef_sdk.model`

输入字段：`run_id`、`plan_id`、`state`、`started_at`、`completed_at`、
`elapsed_seconds`、`total_tasks`、`pending_tasks`、`active_tasks`、
`succeeded_tasks`、`failed_tasks`、`skipped_tasks`、`attempts`、
`repair_turns`、`verification_passes`、`verification_failures`、
`published_outputs`、`tasks`。

`tasks` 是 task ID 到 `TaskRunSummary` 的 immutable mapping；五组 lifecycle
task IDs 互斥并覆盖 `total_tasks`。`completed_tasks` 属性返回 success、failure
与 skipped 的总数。`to_dict()` 与 `from_dict()` 提供严格 JSON round trip。

## WorkflowState

**Canonical FQN（规范完全限定名）**：`clef_sdk.model.WorkflowState`

**Canonical import（规范导入）**：

```python
from clef_sdk.model import WorkflowState
```

Namespace：`clef_sdk.model`

值：

```text
BLOCKED=BLOCKED
CANCELLED=CANCELLED
FAILED=FAILED
PARTIAL=PARTIAL
PENDING=PENDING
RUNNING=RUNNING
SUCCEEDED=SUCCEEDED
TIMED_OUT=TIMED_OUT
```

输出：WorkflowResult 状态。
