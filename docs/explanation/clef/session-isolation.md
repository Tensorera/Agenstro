# Session isolation and recovery

## 1. 两种 session 操作

runtime 明确区分：

- **fresh semantic session**：使用 `session_id=None` 执行完整任务；
- **same-session continuation**：向已有 session 注入修复 prompt，或执行 compact。

不同 TaskRun 之间禁止传递 session identity。任务间上下文只能通过 verified
Artifact 和显式 ContextRef 传播。

## 2. Fresh session

```python
adapter.run(
    prompt,
    workspace=attempt_workspace,
    title=task.id,
    session_id=None,
)
```

成功调用必须返回非空且此前没有注册过的 identity。初始 session 和 recovery
耗尽后创建的 replacement session 都经过同一 uniqueness registry。

## 3. Same-session repair

可恢复错误按以下顺序处理：

```text
failure
  -> repair #1 in same session
  -> ...
  -> repair #5 in same session
  -> native compact in same session
  -> post-compact repair in same session
  -> fresh replacement session
```

每次 continuation 都必须返回原 identity。修复不是“纯格式修复”：它可以在原
contract 允许的范围内重新调用工具、修改输出文件或仅修正 AgentReport。

## 4. Compact

adapter 协议提供：

```python
compact(
    *,
    workspace: Path,
    session_id: str,
) -> AdapterExecution
```

OpenCode 实现使用原生 session summarize API，而不是 `--command compact`。
compact 前后做 workspace snapshot，并检查没有文件变化和工具事件。identity
不一致或 compact 产生副作用属于硬错误。

## 5. OpenCode natural exit

adapter 不接收 timeout 参数。`opencode run` 自己订阅 session 状态并等到 idle；
Clef SDK 等待 CLI 自然返回。

session trace 记录：

- `session_running`；
- `agent_event`；
- `session_tool_result`；
- `session_exited`；
- `session_compaction_started` / `session_compaction_finished`；
- `session_restarted`。

OpenCode/provider 报告的 timeout 会映射为 `backend_timeout`，但 runtime 不创建
或触发计时器。

## 6. Outer retry

replacement session 仍属于同一个 `run_id + task_id + attempt`。只有整个
session recovery 状态机结束后，scheduler 才根据 ErrorInfo.retryable、
ResourcePolicy.max_attempts 和 RetryWorkspaceStrategy 决定是否创建下一个
attempt。

## 7. Artifact context

任务间信息流：

```text
upstream verified Artifact
  -> ArtifactBinding
  -> downstream AgentRequest.inputs
```

长文本上下文可以写入 ArtifactRef，并通过 ContextRef.summary_artifact 进入
下游任务。SessionResult 只发布最终验证通过的 ArtifactRef。
