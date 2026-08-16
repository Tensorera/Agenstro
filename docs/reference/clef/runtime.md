# `clef_sdk.runtime` reference

本卷收录 6 个公共 API 和 3 个公共实体。所有符号均可从 `clef_sdk.runtime` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`ConsoleProgressObserver`](#consoleprogressobserver) | API | public API | 线程安全地输出有界运行摘要行 |
| [`domain_run`](#domain_run) | API | public API | 执行一个 SessionTask |
| [`DomainRunner`](#domainrunner) | API | public API | 执行 TaskRun attempt |
| [`execute_plan`](#execute_plan) | API | public API | 编译并执行 WorkflowPlan |
| [`format_progress_event`](#format_progress_event) | API | public API | 将 typed progress event 格式化为安全单行文本 |
| [`ProgressCallback`](#progresscallback) | Entity | type alias | legacy mapping callback |
| [`ProgressObserver`](#progressobserver) | Entity | type alias | typed progress observer |
| [`RecoveryPolicy`](#recoverypolicy) | Entity | dataclass | TaskRun session 恢复上限 |
| [`WorkflowExecutor`](#workflowexecutor) | API | public API | 调度静态 DAG |

## ConsoleProgressObserver

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.ConsoleProgressObserver`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import ConsoleProgressObserver
```

构造：

```python
ConsoleProgressObserver(
    stream: TextIO | None = None,
    *,
    flush: bool = True,
)
```

执行：

```python
observer(event: RunProgressEvent) -> None
```

输出：在 lock 内把 `format_progress_event(event)` 写入指定 stream；默认使用
`sys.stdout`。输出只含有界摘要字段，不包含完整 prompt、report 或 candidate
text。

## domain_run

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.domain_run`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import domain_run
```

```python
domain_run(
    task: SessionTask,
    *,
    profile: Profile,
    adapter: AgentAdapter | None = None,
    verifier_registry: VerifierRegistry | None = None,
    recovery_policy: RecoveryPolicy | None = None,
) -> SessionResult
```

输入：SessionTask、Profile、可选 adapter 和可选 registry。

输出：attempt 编号为 1 的 `SessionResult`。初次 adapter 调用使用新 session。

## DomainRunner

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.DomainRunner`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import DomainRunner
```

构造：

```python
DomainRunner(
    profile: Profile,
    adapter: AgentAdapter,
    verifier_registry: VerifierRegistry,
    recovery_policy: RecoveryPolicy = RecoveryPolicy(),
)
```

工厂：

```python
DomainRunner.from_profile(
    profile: Profile,
    *,
    adapter: AgentAdapter | None = None,
    verifier_registry: VerifierRegistry | None = None,
    recovery_policy: RecoveryPolicy | None = None,
) -> DomainRunner
```

执行：

```python
run_attempt(
    task: SessionTask,
    *,
    run_id: str,
    attempt: int,
) -> SessionResult
```

输入：运行依赖、SessionTask、run identity 和 attempt 编号。

输出：`SessionResult`。默认恢复顺序是同 session 修复 5 次、compact、再修复
1 次；仍失败时创建一个 replacement session 并重复该有界顺序。

## execute_plan

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.execute_plan`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import execute_plan
```

```python
execute_plan(
    plan: WorkflowPlan,
    *,
    profile: Profile,
    adapter: AgentAdapter | None = None,
    verifier_registry: VerifierRegistry | None = None,
    progress: ProgressCallback | None = None,
    observer: ProgressObserver | None = None,
) -> WorkflowResult
```

输入：WorkflowPlan、Profile、可选 adapter、可选 registry 和可选 progress
callback/typed observer。

输出：带 terminal `WorkflowResult.summary` 的 `WorkflowResult`。

## format_progress_event

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.format_progress_event`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import format_progress_event
```

```python
format_progress_event(event: RunProgressEvent) -> str
```

输入：一条 typed progress event。

输出：包含 sequence、相对耗时、event、task/plan、attempt、task 完成计数和
active task 的单行文本；部分 failure/retry/skip 事件增加有界详情。

## ProgressCallback

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.ProgressCallback`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import ProgressCallback
```

定义：

```python
ProgressCallback = Callable[[dict[str, Any]], None]
```

输入：向后兼容的 progress event mapping。旧事件集合、事件名与 payload 字段
保持不变；它不接收新增 plan/session/verification typed-only 事件，也不含
`summary`。

输出：无。新 UI 优先使用 `ProgressObserver`。

## ProgressObserver

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.ProgressObserver`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import ProgressObserver
```

定义：

```python
ProgressObserver = Callable[[RunProgressEvent], None]
```

输入：严格排序的 typed `RunProgressEvent`。优先读取其聚合 `summary`；事件专属
`data` 可能随错误详情或计划规模增长，不应由 UI 整块渲染。

输出：无。observer 异常被转成 best-effort warning，不改变 workflow 结果。

## RecoveryPolicy

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.RecoveryPolicy`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import RecoveryPolicy
```

Namespace：`clef_sdk.runtime`

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `repairs_before_compact` | `int` | `5` | compact 前同 session 修复次数 |
| `repairs_after_compact` | `int` | `1` | compact 后同 session 修复次数 |
| `max_sessions` | `int` | `2` | 一个 attempt 最多使用的 fresh session 数 |

输出：`DomainRunner` 的有界 session 恢复策略。

## WorkflowExecutor

**Canonical FQN（规范完全限定名）**：`clef_sdk.runtime.WorkflowExecutor`

**Canonical import（规范导入）**：

```python
from clef_sdk.runtime import WorkflowExecutor
```

构造：

```python
WorkflowExecutor(
    profile: Profile,
    runner: DomainRunner,
    progress: ProgressCallback | None = None,
    observer: ProgressObserver | None = None,
)
```

工厂：

```python
WorkflowExecutor.from_profile(
    profile: Profile,
    *,
    adapter: AgentAdapter | None = None,
    verifier_registry: VerifierRegistry | None = None,
    progress: ProgressCallback | None = None,
    observer: ProgressObserver | None = None,
) -> WorkflowExecutor
```

执行：

```python
execute(definition: WorkflowPlan) -> WorkflowResult
```

输入：运行依赖和 WorkflowPlan。

输出：`WorkflowResult`。
