# `clef_sdk.adapters` reference

本卷收录 3 个公共 API 和 5 个公共实体。所有符号均可从 `clef_sdk.adapters` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`AdapterConfigurationError`](#adapterconfigurationerror) | Entity | exception | prompt 发送前发现的非重试配置错误 |
| [`AdapterExecution`](#adapterexecution) | Entity | dataclass | 一次 adapter 调用的观测结果 |
| [`AdapterExitReason`](#adapterexitreason) | Entity | enum | backend 自然退出原因 |
| [`AdapterUsage`](#adapterusage) | Entity | dataclass | adapter 资源用量 |
| [`AgentAdapter`](#agentadapter) | API | public API | 定义 agent transport 的 `run()` 和 `compact()` 协议 |
| [`FakeAdapter`](#fakeadapter) | API | public API | 执行预设响应或回调 |
| [`FakeStep`](#fakestep) | Entity | dataclass | FakeAdapter 队列项 |
| [`OpenCodeAdapter`](#opencodeadapter) | API | public API | 通过 OpenCode CLI 执行 agent session |

## AdapterConfigurationError

**Canonical FQN（规范完全限定名）**：
`clef_sdk.adapters.AdapterConfigurationError`

```python
from clef_sdk.adapters import AdapterConfigurationError
```

字段：`code` 与 JSON-compatible `details`。OpenCodeAdapter 在 fresh turn 前无法
确认模型 catalog、已知模型不存在或 logical effort route 缺失时抛出。标准
runtime 捕获后生成 `retryable=False` 的 REJECTED SessionResult，不发送 prompt。

## AdapterExecution

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.AdapterExecution`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import AdapterExecution
```

Namespace：`clef_sdk.adapters`

输入字段：`session_id`、`text`、`events`、`command`、`return_code`、`stdout`、
`stderr`、`started_at`、`finished_at`、`exit_reason`、`usage`。

输出：一次 adapter 调用的 immutable 观测数据。`succeeded` 属性要求正常退出且
return code 为 0。`timed_out` 是由 `exit_reason == BACKEND_TIMEOUT` 派生的只读
属性，不代表 Clef SDK 设置了计时器。

## AdapterExitReason

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.AdapterExitReason`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import AdapterExitReason
```

Namespace：`clef_sdk.adapters`

```text
COMPLETED=completed
BACKEND_TIMEOUT=backend_timeout
ERROR=error
```

`BACKEND_TIMEOUT` 只表示 OpenCode 或模型 provider 自己报告了 timeout。

## AdapterUsage

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.AdapterUsage`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import AdapterUsage
```

Namespace：`clef_sdk.adapters`

输入字段：`input_tokens`、`output_tokens`、`reasoning_tokens`、
`cache_read_tokens`、`cache_write_tokens`、`cost`。

输出：adapter 资源用量。当前只用于 trace 和 WorkflowResult 观测，不参与
TaskRun 终止或恢复决策。

## AgentAdapter

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.AgentAdapter`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import AgentAdapter
```

导入：

```python
from clef_sdk.adapters import AgentAdapter
```

方法：

```python
run(
    prompt: str,
    *,
    workspace: Path,
    title: str | None = None,
    session_id: str | None = None,
    effort: Effort | None = None,
) -> AdapterExecution

compact(
    *,
    workspace: Path,
    session_id: str,
    effort: Effort | None = None,
) -> AdapterExecution
```

输入：

- `prompt`：完整 agent prompt；
- `workspace`：本次调用的工作目录；
- `title`：可选调用标题；
- `session_id`：同一 attempt 修复或 compact 使用的 session identity；
- `effort`：Clef logical route；不是 OpenCode native variant。

`run()` 和 `compact()` 都等待 backend session 自然结束。接口不提供 wall-clock
timeout；调用方需要时应使用 Python async、线程或独立进程在外层实现取消。

输出：`AdapterExecution`，包含 session identity、文本、事件、退出原因、进程
状态和观测 usage。

## FakeAdapter

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.FakeAdapter`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import FakeAdapter
```

构造：

```python
FakeAdapter(
    steps: Iterable[FakeStep | str | FakeCallback],
)
```

执行：

```python
run(
    prompt: str,
    *,
    workspace: Path,
    title: str | None = None,
    session_id: str | None = None,
    effort: Effort | None = None,
) -> AdapterExecution
```

输入：预设 step 队列和标准 adapter 调用参数。

输出：`AdapterExecution`。`calls` 属性保存调用记录。

## FakeStep

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.FakeStep`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import FakeStep
```

Namespace：`clef_sdk.adapters`

输入字段：

- `response: str | FakeCallback`；
- `session_id: str | None`；
- `return_code: int`；
- `exit_reason: AdapterExitReason = AdapterExitReason.COMPLETED`。

输出：FakeAdapter 队列中的一次响应定义。`return_code` 非零而
`exit_reason=COMPLETED` 时，FakeAdapter 将可观察 exit reason 规范化为 `ERROR`。

## OpenCodeAdapter

**Canonical FQN（规范完全限定名）**：`clef_sdk.adapters.OpenCodeAdapter`

**Canonical import（规范导入）**：

```python
from clef_sdk.adapters import OpenCodeAdapter
```

构造：

```python
OpenCodeAdapter(
    executable: str = "opencode",
    model: str | None = None,
    agent: str | None = None,
    variant: str | None = None,
    attach_url: str | None = None,
    auto_approve: bool = True,
    pure: bool = False,
    inherit_environment: bool = True,
    extra_args: tuple[str, ...] = (),
    environment: dict[str, str] | None = None,
    models: Mapping[str | Effort, ModelRoute] = FrozenDict(),
)
```

执行：

```python
run(
    prompt: str,
    *,
    workspace: Path,
    title: str | None = None,
    session_id: str | None = None,
    effort: Effort | None = None,
) -> AdapterExecution
```

输入：CLI 配置、环境配置和标准 adapter 调用参数。prompt 通过 stdin 传输。

`auto_approve=True` 为 `opencode run` 添加 `--auto`。它只自动批准 OpenCode
原本会询问的权限；OpenCode 配置中的显式 `deny` 仍然有效。Clef 不解析或
重复实现 OpenCode 的 shell、network、edit 等 permission rules。

`run()` 不向 `subprocess.run()` 设置 timeout，而是等待 OpenCode 把 session
推进到 idle 或 error。`compact()` 使用 OpenCode 原生 session summarize API，
并等待当前 session 回到 idle；它不会把 `compact` 误当作 `--command` 的用户命令。

显式 effort 从 `models` 中解析出 concrete model 和 variant，并在 fresh turn 前
通过无 prompt 的 `opencode models` 查询确认可用性。repair、replacement 和
compact 都保留相同 route。catalog 无法确认或 model 缺失时，不执行
`opencode run`。

输出：`AdapterExecution`。
