# 工作流调度

本页解释 ready queue、retry 和失败传播的当前实现。精确函数签名与结果字段以
[API 参考](../../reference/clef-api.md) 和
[实体参考](../../reference/clef-entities.md) 为准。

## 1. 入口

```python
result = execute_plan(
    plan,
    profile=profile,
    adapter=adapter,
    verifier_registry=registry,
    progress=callback,
)
```

`execute_plan()` 创建 WorkflowExecutor 并调用 `execute()`。

## 2. 调度数据

调度器维护：

- compiled plan；
- dependency count；
- dependent task 集合；
- ready queue；
- running future 集合；
- task attempt 列表；
- verified output mapping；
- skipped task 集合；
- resource usage 观测（不参与 TaskRun 终止）；
- workflow trace；
- progress callback。

## 3. Ready queue

task 进入 ready queue 的条件：

```text
all dependencies reached terminal success
AND all bound Artifacts are available
AND concurrency capacity is available
```

ready queue 使用编译拓扑序和 task ID 生成稳定优先级。

## 4. 并发

并发上限来自：

```text
min(
  Profile.runtime.max_concurrency,
  WorkflowPolicies.max_concurrency
)
```

`ResourcePolicy.concurrency_key` 为共享资源建立串行键。已有相同 key 的 task
运行时，scheduler 暂不提交另一个 task；当前实现不创建跨进程锁。

## 5. Task 提交

提交路线：

```text
ready task
  -> bind verified upstream Artifacts
  -> create attempt task
  -> DomainRunner.run_attempt
  -> SessionResult
  -> update task history
  -> publish verified outputs
  -> release dependents
```

每次提交包含 run ID、task ID 和 attempt 编号。

## 6. Retry

retry 条件来自：

- ErrorInfo.category；
- ErrorInfo.retryable；
- verifier 结果；
- ResourcePolicy.max_attempts；
- RuntimeConfig.max_attempts；
- RetryWorkspaceStrategy。

retry 过程：

```text
failed attempt
  -> collect deterministic feedback
  -> allocate next attempt number
  -> allocate fresh session
  -> allocate attempt workspace
  -> submit task
```

retry feedback 包含有界失败 attempt 和 required check 摘要。

## 7. 依赖传播

`WorkflowPolicies.failure_policy` 当前会被验证和序列化，但 v0.1 scheduler
尚未读取该字段。因此 `FAIL_FAST`、`SKIP_DEPENDENTS` 和 `CONTINUE` 目前是保留
的策略词汇，不应被当作已经实施的分支行为。

当前传播规则是：

- 依赖失败 task 的节点进入 skipped 集合；
- 没有失败依赖的独立分支可以继续；
- `WorkflowPolicies.fail_fast` 或 `RuntimeConfig.fail_fast` 任一为 `True` 时，
  第一次 task 失败后所有尚未提交的 task 都会被 skipped。

## 8. 两级运行摘要

SDK 同时提供实时事件与最终快照。最直接的 console 用法是：

```python
from clef_sdk import ConsoleProgressObserver, execute_plan

result = execute_plan(
    plan,
    profile=profile,
    adapter=adapter,
    verifier_registry=registry,
    observer=ConsoleProgressObserver(),
)

assert result.summary is not None
print(result.summary.to_dict())
```

`ConsoleProgressObserver` 每个事件输出一行经过裁剪的摘要，包含 sequence、相对
耗时、事件、task/attempt、完成数、成功/失败/跳过数和 active tasks。verification
失败行还会显示 required check 数与有界名称；它不会打印 prompt、完整 report 或
candidate text。

需要结构化 UI 时，传入 typed observer：

```python
from clef_sdk import RunProgressEvent, execute_plan

def observe(event: RunProgressEvent) -> None:
    task = event.summary.tasks.get(event.task_id) if event.task_id else None
    render(event.event, event.scope, task, event.summary)

result = execute_plan(
    plan,
    profile=profile,
    observer=observe,
)
```

`RunProgressEvent` 提供稳定的 `schema_version`、`sequence`、`timestamp`、
`elapsed_seconds`、`run_id`、`plan_id`、`event`、`scope`、可选
`task_id`/`attempt`、当前 `summary` 与事件专属 `data`。`summary` 是供 UI
消费的聚合状态；`data` 可能包含路径、错误详情或随计划规模增长的列表，不应
未经选择直接渲染。scope 是 `plan`、
`workflow`、`task`、`attempt`、`agent`、`verification` 或 `publication`。
typed event 集合如下：

| Scope | Events |
| --- | --- |
| plan | `plan_received`, `plan_compile_started`, `plan_compiled`, `plan_compile_failed` |
| workflow | `workflow_started`, `workflow_completed` |
| task | `task_scheduled`, `task_succeeded`, `task_failed`, `task_skipped`, `task_binding_failed`, `task_deadlocked`, `task_executor_error` |
| attempt/retry | `attempt_started`, `attempt_completed`, `attempt_failed`, `attempt_precondition_failed`, `retry_scheduled`, `retry_suppressed`, `retry_workspace_isolated`, `retry_workspace_published` |
| agent | `agent_session_started`, `agent_session_completed`, `agent_session_restarted`, `repair_started`, `agent_compaction_started`, `agent_compaction_completed`, `agent_recovery_exhausted` |
| verification | `verification_passed`, `verification_failed` |
| publication | `artifact_published`, `artifact_publication_failed`, `attempt_published` |

`task_scheduled` 表示 scheduler 已接受该 task 并创建运行项，不表示 agent 已经
返回结果。

每个 `WorkflowRunSummary` 汇总：

- workflow state、开始/完成时间和相对耗时；
- pending、active、succeeded、failed、skipped task IDs；
- attempt、repair、verification pass/failure 和 published output 计数；
- 每个 task 的 `TaskRunSummary`，含当前
  `PENDING/SCHEDULED/RUNNING/VERIFYING/REPAIRING/PUBLISHING` 或 terminal
  状态、attempt/session/turn、计数和耗时。

终态快照存入 `WorkflowResult.summary`，因此 console 或 observer 不是状态
authority。原有 `progress=` mapping callback 保持旧事件集合、事件名和 payload
字段不变，不接收新增的 plan/session/verification typed-only 事件。新代码优先
使用 `observer=` 的 `RunProgressEvent`。

## 9. 摘要与原始 JSONL

每个 high-level progress event 都对应一个先追加到 workflow
`workflow.jsonl` 的持久 record。JSONL 的 `data.progress` 保存 schema version、
typed event 名、scope、attempt、elapsed time 和当时的完整 summary。typed
observer 事件与该 record 共用 sequence，但可以使用更清楚的 typed 名称，例如 raw legacy
`task_started` 对应 typed `attempt_started`；不要按事件名称连接两种表示。每个
TaskRun attempt 的原始 trace 继续单独保存
agent/session/candidate/verifier/artifact 证据；其中与运行摘要相关的选定事件会
被映射到 workflow trace，但原始证据不会被 summary 替代。

事件投递通过一个总序列锁定，即使 task 并发执行，typed observer 看到的
sequence 仍与对应 workflow JSONL record 一致。observer、legacy progress
callback 或底层 trace observer 抛出的异常不会改变 workflow 结果；框架会尽力追加
`progress_callback_failed` warning。

## 10. WorkflowResult

调度结束后生成：

- workflow state；
- task_results；
- 具名 workflow outputs；
- skipped_tasks；
- ResourceUsage；
- ExecutionTraceRef；
- ErrorInfo；
- WorkflowRunSummary。
