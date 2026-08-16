# Clef SDK 概览

本页解释 `clef_sdk` 包的用途和主要能力。精确 API 与字段以
Reference 页面为准。

## 1. 包说明

Clef SDK 是一个面向 agent 工作流的 Python 运行框架。包名为
`clef_sdk`，运行环境为 Python 3.12。

框架接收结构化任务定义和运行 Profile，完成计划编译、依赖调度、agent 调用、
结果验证、Artifact 发布、运行追踪和资源汇总。

领域包负责输入发现、prompt、WorkflowPlan 构造、领域 verifier 和交付布局。
Clef SDK 提供稳定的基础类型与执行接口。

仓库中的 Tactus Runtime 是独立安装的第二个项目，负责 compose/run
状态机和 Motivo Studio。Clef SDK 不依赖 Tactus，Tactus
也不把静态 DAG 状态混入自己的 occurrence。这个边界保证两个项目都能独立
安装、测试和演进。参见
[Tactus architecture](tactus-runtime-architecture.md)。

## 2. 功能

### 2.1 任务模型

- `SessionTask` 描述一个 agent 任务；
- `DomainContract` 描述输入、输出、effect 意图、调度和验证规则；
- `WorkflowPlan` 描述静态 DAG；
- `ArtifactBinding` 连接上游输出和下游输入；
- frozen dataclass 保持运行定义稳定；
- canonical JSON 提供稳定序列化。

### 2.2 计划编译

- Profile 绑定；
- DAG 检查；
- 确定性拓扑排序；
- Artifact kind 检查；
- 路径规范化；
- workspace 边界检查；
- plan digest；
- 结构化验证报告。

### 2.3 运行调度

- ready-queue 调度；
- 有限并发；
- 依赖状态传播；
- task retry；
- attempt workspace；
- typed workflow progress observer 与 legacy callback；
- task/workflow 实时摘要和 `WorkflowResult.summary`；
- thread-safe console 摘要；
- 资源用量汇总。

### 2.4 Session 隔离

- 每个 TaskRun 初次调用使用新 session；
- 每个 retry attempt 使用新 session；
- runner 记录观察到的 session identity；
- session identity 唯一性检查；
- 同一 attempt 的执行、工具、输出和协议错误优先在当前 session 修复；
- 五次修复后 compact，一次 compact 后修复仍失败才启用 replacement session；
- retry feedback 使用有界结构化数据。

### 2.5 验证

- Artifact 约束转换；
- verifier registry；
- 文件存在验证；
- digest 验证；
- size 验证；
- media type 验证；
- JSON Schema Draft 2020-12 验证；
- Markdown 本地资源验证；
- 可见文本长度验证；
- 命令退出状态验证；
- 调用方 verifier 注入。

### 2.6 副作用观测

- workspace snapshot；
- create、modify、move、delete 检测；
- 声明副作用匹配元数据；
- adapter 调用前后审计；
- verifier 调用前后审计；
- trace 事件记录。

副作用匹配只用于结果观测，不用于批准或拒绝 OpenCode 工具。工具权限完全由
OpenCode 原生 `permission` 配置执行。

### 2.7 存储

- Content Addressed Store；
- deterministic Manifest；
- publish-once；
- JSONL trace；
- conservative cache；
- Artifact 稳定排序；
- digest 复核。

### 2.8 Adapter

- `AgentAdapter` 协议；
- `OpenCodeAdapter` 进程适配；
- `FakeAdapter` 确定性执行；
- stdin prompt 传输；
- 等待 OpenCode session 自然 idle；
- 同 session repair 和原生 compact；
- stdout 和 stderr 捕获；
- session identity 提取；
- backend exit reason 与 usage 观测。

### 2.9 Profile

- TOML 配置；
- adapter 配置；
- runtime 配置；
- workspace 配置；
- storage 配置；
- 路径解析；
- 文件系统检查；
- Profile digest；
- 配置脱敏。

## 3. 设计理念

### 3.1 Contract-driven

任务输入、输出、执行意图、调度和验证规则进入 `DomainContract`。运行时检查
Artifact 与确定性验收条件；OpenCode 自己负责工具权限。

### 3.2 Deterministic boundary

计划、Profile、Artifact、Manifest 和协议消息使用稳定序列化。排序键由计划字段
生成。digest 标识冻结内容。

### 3.3 Context isolation

TaskRun 使用独立 session。下游任务通过已经验证的 Artifact 接收信息。retry
通过结构化验证反馈接收修复条件。

### 3.4 Observable effects

运行时采集 workspace snapshot，并生成稳定的 `WorkspaceDiff`。副作用记录进入
`SessionResult` 和 trace。

### 3.5 Verified publication

Artifact 依次经过声明解析、路径检查、verifier chain、CAS 和稳定
槽位发布。

### 3.6 Dependency injection

Profile、adapter 和 verifier registry 通过函数参数进入 runtime。调用方在进程
边界组装依赖。

### 3.7 Domain extension

领域包使用公开 model 构造任务和计划，使用 `VerifierRegistry.register()` 注册
领域验证器，使用 storage API 构造领域 Artifact 布局。

## 4. 包结构

```text
clef_sdk/
  adapters/       agent transport
  compiler/       plan normalization and validation
  model/          entities and enums
  profiles/       TOML schema and loader
  protocol/       AgentRequest and AgentReport codec
  runtime/        TaskRun and DAG scheduler
  storage/        snapshot, CAS, trace, cache, manifest
  verification/   verifier registry and built-ins
```

## 5. 核心执行路线

```text
Profile
  -> WorkflowPlan
  -> compile_plan
  -> CompiledPlan
  -> WorkflowExecutor
  -> DomainRunner
  -> AgentAdapter
  -> AgentReport
  -> workspace audit
  -> verifier chain
  -> CAS and stable Artifact
  -> WorkflowResult
```

## 6. 相关文档

- [First Clef SDK workflow](../tutorials/clef-first-workflow.md)
- [Clef API reference](../reference/clef-api.md)
- [Clef entity reference](../reference/clef-entities.md)
- [Clef architecture](clef/overview.md)
- [Development environment](../how-to/develop.md)
