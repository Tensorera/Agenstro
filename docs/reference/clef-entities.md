# Clef SDK entity index

本索引完整列出 Clef SDK 的 109 个公共实体。详细说明按公共命名空间拆分到 8 个 reference volume；每个符号名称都直接链接到对应详情。

## Namespace volumes

| Namespace | API | Entity | Reference volume |
| --- | ---: | ---: | --- |
| `clef_sdk.adapters` | 3 | 5 | [open](clef/adapters.md) |
| `clef_sdk.compiler` | 4 | 4 | [open](clef/compiler.md) |
| `clef_sdk.model` | 5 | 51 | [open](clef/model.md) |
| `clef_sdk.profiles` | 4 | 9 | [open](clef/profiles.md) |
| `clef_sdk.protocol` | 7 | 11 | [open](clef/protocol.md) |
| `clef_sdk.runtime` | 6 | 3 | [open](clef/runtime.md) |
| `clef_sdk.storage` | 10 | 24 | [open](clef/storage.md) |
| `clef_sdk.verification` | 10 | 2 | [open](clef/verification.md) |

## Entity symbols

名称按 `0-9a-z` 排序。

| Entity | 类型 | Namespace | 一行说明 |
| --- | --- | --- | --- |
| [`AdapterConfigurationError`](clef/adapters.md#adapterconfigurationerror) | exception | [`clef_sdk.adapters`](clef/adapters.md) | prompt 发送前发现的非重试配置错误 |
| [`AdapterConfig`](clef/profiles.md#adapterconfig) | dataclass | [`clef_sdk.profiles`](clef/profiles.md) | OpenCode adapter 配置 |
| [`AdapterExecution`](clef/adapters.md#adapterexecution) | dataclass | [`clef_sdk.adapters`](clef/adapters.md) | 一次 adapter 调用的观测结果 |
| [`AdapterExitReason`](clef/adapters.md#adapterexitreason) | enum | [`clef_sdk.adapters`](clef/adapters.md) | backend 自然退出原因 |
| [`AdapterUsage`](clef/adapters.md#adapterusage) | dataclass | [`clef_sdk.adapters`](clef/adapters.md) | adapter 资源用量 |
| [`AgentReport`](clef/protocol.md#agentreport) | dataclass | [`clef_sdk.protocol`](clef/protocol.md) | agent 返回协议对象 |
| [`AgentRequest`](clef/protocol.md#agentrequest) | dataclass | [`clef_sdk.protocol`](clef/protocol.md) | runtime 发出的协议对象 |
| [`ArtifactBinding`](clef/model.md#artifactbinding) | dataclass | [`clef_sdk.model`](clef/model.md) | task 输出到 task 输入的绑定 |
| [`ArtifactChange`](clef/model.md#artifactchange) | dataclass | [`clef_sdk.model`](clef/model.md) | TaskRun 文件变化记录 |
| [`ArtifactChangeKind`](clef/model.md#artifactchangekind) | enum | [`clef_sdk.model`](clef/model.md) | TaskRun 文件变化类型 |
| [`ArtifactClaim`](clef/model.md#artifactclaim) | dataclass | [`clef_sdk.model`](clef/model.md) | AgentReport 中的输出声明 |
| [`ArtifactConstraint`](clef/model.md#artifactconstraint) | dataclass | [`clef_sdk.model`](clef/model.md) | ArtifactSpec 的确定性约束 |
| [`ArtifactConstraintKind`](clef/model.md#artifactconstraintkind) | enum | [`clef_sdk.model`](clef/model.md) | Artifact 约束类型 |
| [`ArtifactKind`](clef/model.md#artifactkind) | enum | [`clef_sdk.model`](clef/model.md) | Artifact 数据类型 |
| [`ArtifactMutation`](clef/model.md#artifactmutation) | dataclass | [`clef_sdk.model`](clef/model.md) | artifact tree 的一个净变化 |
| [`ArtifactMutationKind`](clef/model.md#artifactmutationkind) | enum | [`clef_sdk.model`](clef/model.md) | create、update 或 delete |
| [`ArtifactOrderKey`](clef/storage.md#artifactorderkey) | dataclass | [`clef_sdk.storage`](clef/storage.md) | Manifest 稳定排序键 |
| [`ArtifactProvenance`](clef/model.md#artifactprovenance) | dataclass | [`clef_sdk.model`](clef/model.md) | Artifact 来源信息 |
| [`ArtifactRef`](clef/model.md#artifactref) | dataclass | [`clef_sdk.model`](clef/model.md) | 已存在 Artifact 引用 |
| [`ArtifactSpec`](clef/model.md#artifactspec) | dataclass | [`clef_sdk.model`](clef/model.md) | task 预期输出定义 |
| [`CacheConflictError`](clef/storage.md#cacheconflicterror) | exception | [`clef_sdk.storage`](clef/storage.md) | 缓存内容冲突 |
| [`CacheCorruptionError`](clef/storage.md#cachecorruptionerror) | exception | [`clef_sdk.storage`](clef/storage.md) | 缓存内容完整性错误 |
| [`CacheEligibility`](clef/storage.md#cacheeligibility) | dataclass | [`clef_sdk.storage`](clef/storage.md) | 缓存写入资格 |
| [`CacheError`](clef/storage.md#cacheerror) | exception | [`clef_sdk.storage`](clef/storage.md) | 缓存错误基类 |
| [`CacheHit`](clef/storage.md#cachehit) | dataclass | [`clef_sdk.storage`](clef/storage.md) | 缓存命中结果 |
| [`CacheIdentity`](clef/storage.md#cacheidentity) | dataclass | [`clef_sdk.storage`](clef/storage.md) | 缓存身份字段集合 |
| [`CacheNotEligibleError`](clef/storage.md#cachenoteligibleerror) | exception | [`clef_sdk.storage`](clef/storage.md) | 缓存资格错误 |
| [`CASCorruptionError`](clef/storage.md#cascorruptionerror) | exception | [`clef_sdk.storage`](clef/storage.md) | CAS 内容完整性错误 |
| [`CASError`](clef/storage.md#caserror) | exception | [`clef_sdk.storage`](clef/storage.md) | CAS 错误基类 |
| [`CASObject`](clef/storage.md#casobject) | dataclass | [`clef_sdk.storage`](clef/storage.md) | CAS 对象描述 |
| [`ChangeKind`](clef/storage.md#changekind) | enum | [`clef_sdk.storage`](clef/storage.md) | WorkspaceDiff 变化类型 |
| [`CheckResult`](clef/model.md#checkresult) | dataclass | [`clef_sdk.model`](clef/model.md) | 单个 verifier 结果 |
| [`CheckStatus`](clef/model.md#checkstatus) | enum | [`clef_sdk.model`](clef/model.md) | verifier 状态 |
| [`CompiledPlan`](clef/compiler.md#compiledplan) | dataclass | [`clef_sdk.compiler`](clef/compiler.md) | WorkflowPlan 编译结果 |
| [`ContextRef`](clef/model.md#contextref) | dataclass | [`clef_sdk.model`](clef/model.md) | 显式上下文引用 |
| [`DomainContract`](clef/model.md#domaincontract) | dataclass | [`clef_sdk.model`](clef/model.md) | task 输入输出和运行约束 |
| [`EffectKind`](clef/model.md#effectkind) | enum | [`clef_sdk.model`](clef/model.md) | 副作用类型 |
| [`EffectPolicy`](clef/model.md#effectpolicy) | dataclass | [`clef_sdk.model`](clef/model.md) | 发给 agent 的 effect 意图 |
| [`EffectRule`](clef/model.md#effectrule) | dataclass | [`clef_sdk.model`](clef/model.md) | 一条副作用路径规则 |
| [`Effort`](clef/model.md#effort) | enum | [`clef_sdk.model`](clef/model.md) | task 的逻辑模型路由档位 |
| [`EntryKind`](clef/storage.md#entrykind) | enum | [`clef_sdk.storage`](clef/storage.md) | SnapshotEntry 类型 |
| [`ErrorCategory`](clef/model.md#errorcategory) | enum | [`clef_sdk.model`](clef/model.md) | 运行错误分类 |
| [`ErrorInfo`](clef/model.md#errorinfo) | dataclass | [`clef_sdk.model`](clef/model.md) | 结构化错误 |
| [`ExecutionTraceRef`](clef/model.md#executiontraceref) | dataclass | [`clef_sdk.model`](clef/model.md) | trace 文件引用 |
| [`FailurePolicy`](clef/model.md#failurepolicy) | enum | [`clef_sdk.model`](clef/model.md) | 已序列化、当前 scheduler 尚未消费的失败策略词汇 |
| [`FakeStep`](clef/adapters.md#fakestep) | dataclass | [`clef_sdk.adapters`](clef/adapters.md) | FakeAdapter 队列项 |
| [`FrozenDict`](clef/model.md#frozendict) | mapping | [`clef_sdk.model`](clef/model.md) | immutable mapping |
| [`JsonModel`](clef/model.md#jsonmodel) | base class | [`clef_sdk.model`](clef/model.md) | 严格 JSON entity 基类 |
| [`ManifestConflictError`](clef/storage.md#manifestconflicterror) | exception | [`clef_sdk.storage`](clef/storage.md) | Manifest 发布冲突 |
| [`ManifestEntry`](clef/storage.md#manifestentry) | dataclass | [`clef_sdk.storage`](clef/storage.md) | Manifest Artifact 条目 |
| [`ManifestError`](clef/storage.md#manifesterror) | exception | [`clef_sdk.storage`](clef/storage.md) | Manifest 错误基类 |
| [`ManifestWriteResult`](clef/storage.md#manifestwriteresult) | dataclass | [`clef_sdk.storage`](clef/storage.md) | Manifest 写入结果 |
| [`ModelDecodeError`](clef/model.md#modeldecodeerror) | exception | [`clef_sdk.model`](clef/model.md) | model 解码错误 |
| [`ModelError`](clef/model.md#modelerror) | exception | [`clef_sdk.model`](clef/model.md) | model 错误基类 |
| [`ModelRoute`](clef/profiles.md#modelroute) | dataclass | [`clef_sdk.profiles`](clef/profiles.md) | logical effort 对应的 OpenCode 模型路由 |
| [`ModelValidationError`](clef/model.md#modelvalidationerror) | exception | [`clef_sdk.model`](clef/model.md) | model 字段校验错误 |
| [`PlanIssue`](clef/compiler.md#planissue) | dataclass | [`clef_sdk.compiler`](clef/compiler.md) | 计划静态检查问题 |
| [`PlanValidationReport`](clef/compiler.md#planvalidationreport) | dataclass | [`clef_sdk.compiler`](clef/compiler.md) | 计划静态检查报告 |
| [`Profile`](clef/profiles.md#profile) | dataclass | [`clef_sdk.profiles`](clef/profiles.md) | 完整运行配置 |
| [`ProfileError`](clef/profiles.md#profileerror) | exception | [`clef_sdk.profiles`](clef/profiles.md) | Profile 错误基类 |
| [`ProfilePathError`](clef/profiles.md#profilepatherror) | exception | [`clef_sdk.profiles`](clef/profiles.md) | Profile 路径错误 |
| [`ProfileValidationError`](clef/profiles.md#profilevalidationerror) | exception | [`clef_sdk.profiles`](clef/profiles.md) | Profile 字段校验错误 |
| [`Prompt`](clef/model.md#prompt) | dataclass | [`clef_sdk.model`](clef/model.md) | 一条 prompt 消息 |
| [`PromptRole`](clef/model.md#promptrole) | enum | [`clef_sdk.model`](clef/model.md) | prompt 角色 |
| [`PROTOCOL_VERSION`](clef/protocol.md#protocol_version) | constant | [`clef_sdk.protocol`](clef/protocol.md) | 当前协议版本 |
| [`ProgressCallback`](clef/runtime.md#progresscallback) | type alias | [`clef_sdk.runtime`](clef/runtime.md) | legacy mapping callback |
| [`ProgressObserver`](clef/runtime.md#progressobserver) | type alias | [`clef_sdk.runtime`](clef/runtime.md) | typed progress observer |
| [`ProtocolCorrelationError`](clef/protocol.md#protocolcorrelationerror) | exception | [`clef_sdk.protocol`](clef/protocol.md) | 协议关联错误 |
| [`ProtocolDecodeError`](clef/protocol.md#protocoldecodeerror) | exception | [`clef_sdk.protocol`](clef/protocol.md) | 协议解码错误 |
| [`ProtocolError`](clef/protocol.md#protocolerror) | exception | [`clef_sdk.protocol`](clef/protocol.md) | 协议错误基类 |
| [`ProtocolValidationError`](clef/protocol.md#protocolvalidationerror) | exception | [`clef_sdk.protocol`](clef/protocol.md) | 协议字段校验错误 |
| [`RecoveryPolicy`](clef/runtime.md#recoverypolicy) | dataclass | [`clef_sdk.runtime`](clef/runtime.md) | TaskRun session 恢复上限 |
| [`REPORT_BEGIN_SENTINEL`](clef/protocol.md#report_begin_sentinel) | constant | [`clef_sdk.protocol`](clef/protocol.md) | AgentReport 起始标记 |
| [`REPORT_END_SENTINEL`](clef/protocol.md#report_end_sentinel) | constant | [`clef_sdk.protocol`](clef/protocol.md) | AgentReport 结束标记 |
| [`ResourcePolicy`](clef/model.md#resourcepolicy) | dataclass | [`clef_sdk.model`](clef/model.md) | task attempt、cache 和 retry 配置 |
| [`ResourceUsage`](clef/model.md#resourceusage) | dataclass | [`clef_sdk.model`](clef/model.md) | workflow 聚合资源用量 |
| [`RetryWorkspaceStrategy`](clef/model.md#retryworkspacestrategy) | enum | [`clef_sdk.model`](clef/model.md) | retry workspace 策略 |
| [`RuleSpec`](clef/model.md#rulespec) | dataclass | [`clef_sdk.model`](clef/model.md) | precondition 或 postcondition 定义 |
| [`RunProgressEvent`](clef/model.md#runprogressevent) | dataclass | [`clef_sdk.model`](clef/model.md) | 一条 typed workflow 运行摘要事件 |
| [`RunState`](clef/model.md#runstate) | enum | [`clef_sdk.model`](clef/model.md) | SessionResult 状态 |
| [`RuntimeConfig`](clef/profiles.md#runtimeconfig) | dataclass | [`clef_sdk.profiles`](clef/profiles.md) | scheduler 配置 |
| [`SessionResult`](clef/model.md#sessionresult) | dataclass | [`clef_sdk.model`](clef/model.md) | TaskRun attempt 结果 |
| [`SessionTask`](clef/model.md#sessiontask) | dataclass | [`clef_sdk.model`](clef/model.md) | agent 任务定义 |
| [`SnapshotEntry`](clef/storage.md#snapshotentry) | dataclass | [`clef_sdk.storage`](clef/storage.md) | workspace 单路径快照 |
| [`SnapshotError`](clef/storage.md#snapshoterror) | exception | [`clef_sdk.storage`](clef/storage.md) | workspace snapshot 错误 |
| [`StorageConfig`](clef/profiles.md#storageconfig) | dataclass | [`clef_sdk.profiles`](clef/profiles.md) | 框架状态目录配置 |
| [`SUPPORTED_PROTOCOL_VERSIONS`](clef/protocol.md#supported_protocol_versions) | constant | [`clef_sdk.protocol`](clef/protocol.md) | 支持的协议版本集合 |
| [`TASK_RESULT_SCHEMA`](clef/model.md#taskresultenvelope) | constant | [`clef_sdk.model`](clef/model.md) | TaskResultEnvelope v1 schema tag |
| [`TaskInput`](clef/model.md#taskinput) | type alias | [`clef_sdk.model`](clef/model.md) | task 输入联合类型 |
| [`TaskProgressState`](clef/model.md#taskprogressstate) | enum | [`clef_sdk.model`](clef/model.md) | task 的实时调度阶段 |
| [`TaskResultEnvelope`](clef/model.md#taskresultenvelope) | dataclass | [`clef_sdk.model`](clef/model.md) | domain JSON 与实际 artifact 变化 |
| [`TaskRunSummary`](clef/model.md#taskrunsummary) | dataclass | [`clef_sdk.model`](clef/model.md) | 一个 task 的当前运行摘要 |
| [`TraceError`](clef/storage.md#traceerror) | exception | [`clef_sdk.storage`](clef/storage.md) | trace 写入错误 |
| [`TraceEvent`](clef/storage.md#traceevent) | dataclass | [`clef_sdk.storage`](clef/storage.md) | 一条 trace 事件 |
| [`UnsupportedProtocolVersion`](clef/protocol.md#unsupportedprotocolversion) | exception | [`clef_sdk.protocol`](clef/protocol.md) | 协议版本错误 |
| [`VerificationContext`](clef/verification.md#verificationcontext) | dataclass | [`clef_sdk.verification`](clef/verification.md) | verifier 可信上下文 |
| [`VerificationReport`](clef/model.md#verificationreport) | dataclass | [`clef_sdk.model`](clef/model.md) | verifier chain 聚合结果 |
| [`Verifier`](clef/verification.md#verifier) | type alias | [`clef_sdk.verification`](clef/verification.md) | verifier callable 类型 |
| [`VerifierSpec`](clef/model.md#verifierspec) | dataclass | [`clef_sdk.model`](clef/model.md) | verifier 调用定义 |
| [`WorkflowCompileError`](clef/compiler.md#workflowcompileerror) | exception | [`clef_sdk.compiler`](clef/compiler.md) | WorkflowPlan 编译错误 |
| [`WorkflowPlan`](clef/model.md#workflowplan) | dataclass | [`clef_sdk.model`](clef/model.md) | 静态 DAG 定义 |
| [`WorkflowPolicies`](clef/model.md#workflowpolicies) | dataclass | [`clef_sdk.model`](clef/model.md) | workflow 调度策略 |
| [`WorkflowResult`](clef/model.md#workflowresult) | dataclass | [`clef_sdk.model`](clef/model.md) | workflow 执行结果 |
| [`WorkflowRunSummary`](clef/model.md#workflowrunsummary) | dataclass | [`clef_sdk.model`](clef/model.md) | workflow 的实时或终态摘要 |
| [`WorkflowState`](clef/model.md#workflowstate) | enum | [`clef_sdk.model`](clef/model.md) | workflow 状态 |
| [`WorkspaceChange`](clef/storage.md#workspacechange) | dataclass | [`clef_sdk.storage`](clef/storage.md) | workspace 单路径变化 |
| [`WorkspaceConfig`](clef/profiles.md#workspaceconfig) | dataclass | [`clef_sdk.profiles`](clef/profiles.md) | workspace 能力配置 |
| [`WorkspaceDiff`](clef/storage.md#workspacediff) | dataclass | [`clef_sdk.storage`](clef/storage.md) | workspace 变化集合 |
| [`WorkspaceSnapshot`](clef/storage.md#workspacesnapshot) | dataclass | [`clef_sdk.storage`](clef/storage.md) | workspace 文件树快照 |
