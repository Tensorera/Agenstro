# Clef SDK 架构

本页解释 Clef SDK 的系统边界、数据流和身份模型。精确的 API
签名与字段定义以 [API 参考](../../reference/clef-api.md) 和
[实体参考](../../reference/clef-entities.md) 为准。

## 1. 系统目标

Clef SDK 为 agent 工作流提供结构化执行内核。系统接收 Profile、
SessionTask 和 WorkflowPlan，输出 SessionResult、WorkflowResult、Artifact 和
trace。

核心架构属性：

- 静态计划；
- 静态计划编译与无 prompt 模型 catalog 校验；
- session 隔离；
- workspace 观测；
- Contract 验证；
- Artifact 驱动的数据流；
- 本地持久化；
- 调用方扩展。

## 2. 系统边界

```text
caller
  input discovery
  domain prompt
  domain schema
  domain verifier
  artifact layout
        |
        v
Clef SDK
  model
  profile
  compiler
  scheduler
  task runner
  protocol
  adapter
  verification
  storage
        |
        v
agent backend
```

调用方通过公开 Entity 和 API 进入系统。agent backend 通过 `AgentAdapter` 进入
系统。

## 3. 主数据流

```text
profile.toml
  -> load_profile
  -> Profile

domain inputs
  -> SessionTask
  -> WorkflowPlan

WorkflowPlan + Profile
  -> compile_plan
  -> CompiledPlan

CompiledPlan
  -> WorkflowExecutor
  -> ready queue
  -> DomainRunner
  -> AgentRequest
  -> AgentAdapter
  -> AgentReport
  -> workspace audit
  -> verifier chain
  -> CAS
  -> stable Artifact
  -> SessionResult
  -> WorkflowResult
```

## 4. 控制流

计划捕获与静态验证发生在 `compile_plan()` 中；`CAPTURED` 和 `VALIDATED`
不是运行状态枚举值。运行结果使用两组有限词汇：

| 范围 | 非终态 | 终态 |
| --- | --- | --- |
| TaskRun (`RunState`) | `PENDING`, `READY`, `RUNNING`, `VERIFYING` | `SUCCEEDED`, `BLOCKED`, `WAITING_INPUT`, `FAILED`, `CANCELLED`, `TIMED_OUT`, `REJECTED` |
| Workflow (`WorkflowState`) | `PENDING`, `RUNNING` | `SUCCEEDED`, `PARTIAL`, `BLOCKED`, `FAILED`, `CANCELLED`, `TIMED_OUT` |

这些枚举定义结果词汇，不表示每个值都会在一次执行中依次持久化。结构化
`ErrorInfo` 记录 category、code、message、retryable、details 和 cause。

## 5. 稳定身份

| 对象 | 身份来源 |
| --- | --- |
| Profile | 脱敏 canonical 配置 digest |
| WorkflowPlan | plan entity 与 Profile digest |
| TaskRun | run ID、task ID、attempt |
| Session | adapter 返回的 session identity |
| Artifact | 计划槽位与内容 digest |
| CASObject | 内容 SHA-256 |
| ManifestEntry | artifact ID 与 ArtifactOrderKey |
| TraceEvent | run ID 与单 writer sequence |

## 6. 架构文档

1. [模块](modules.md)
2. [计划编译](compilation.md)
3. [工作流调度](scheduling.md)
4. [TaskRun](task-run.md)
5. [验证与存储](verification-and-storage.md)
6. [Agent 协议](protocol.md)
7. [Session 隔离](session-isolation.md)
8. [Artifact 排序](artifact-ordering.md)
9. [Profile 边界](profile-boundaries.md)
