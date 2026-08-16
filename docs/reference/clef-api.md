# Clef SDK API index

本索引完整列出 Clef SDK 的 49 个公共 API。详细说明按公共命名空间拆分到 8 个 reference volume；每个符号名称都直接链接到对应详情。

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

## API symbols

名称按 `0-9a-z` 排序。

| API | Namespace | 一行说明 |
| --- | --- | --- |
| [`AgentAdapter`](clef/adapters.md#agentadapter) | [`clef_sdk.adapters`](clef/adapters.md) | 定义 agent transport 的 `run()` 和 `compact()` 协议 |
| [`canonical_json_dumps`](clef/model.md#canonical_json_dumps) | [`clef_sdk.model`](clef/model.md) | 将 JSON 值编码为规范字符串 |
| [`canonical_json_loads`](clef/model.md#canonical_json_loads) | [`clef_sdk.model`](clef/model.md) | 将严格 JSON 字符串解码为 Python 值 |
| [`check_json_schema`](clef/verification.md#check_json_schema) | [`clef_sdk.verification`](clef/verification.md) | 用 Draft 2020-12 meta-schema 检查 Schema |
| [`compile_plan`](clef/compiler.md#compile_plan) | [`clef_sdk.compiler`](clef/compiler.md) | 编译 WorkflowPlan 并生成 CompiledPlan |
| [`compile_workflow`](clef/compiler.md#compile_workflow) | [`clef_sdk.compiler`](clef/compiler.md) | 编译 WorkflowPlan 并返回规范化计划 |
| [`ConservativeCache`](clef/storage.md#conservativecache) | [`clef_sdk.storage`](clef/storage.md) | 管理带资格检查的本地结果缓存 |
| [`ConsoleProgressObserver`](clef/runtime.md#consoleprogressobserver) | [`clef_sdk.runtime`](clef/runtime.md) | 线程安全地输出有界运行摘要行 |
| [`ContentAddressedStore`](clef/storage.md#contentaddressedstore) | [`clef_sdk.storage`](clef/storage.md) | 按 SHA-256 保存和读取字节对象 |
| [`decode_report_envelope`](clef/protocol.md#decode_report_envelope) | [`clef_sdk.protocol`](clef/protocol.md) | 从 sentinel envelope 解码 AgentReport |
| [`decode_report_json`](clef/protocol.md#decode_report_json) | [`clef_sdk.protocol`](clef/protocol.md) | 从严格 JSON 解码 AgentReport |
| [`decode_request`](clef/protocol.md#decode_request) | [`clef_sdk.protocol`](clef/protocol.md) | 从严格 JSON 解码 AgentRequest |
| [`default_profiles_dir`](clef/profiles.md#default_profiles_dir) | [`clef_sdk.profiles`](clef/profiles.md) | 计算平台默认 Profile 目录 |
| [`default_registry`](clef/verification.md#default_registry) | [`clef_sdk.verification`](clef/verification.md) | 创建通用 verifier registry |
| [`DeterministicManifestWriter`](clef/storage.md#deterministicmanifestwriter) | [`clef_sdk.storage`](clef/storage.md) | 写入确定性 Artifact Manifest |
| [`diff_snapshots`](clef/storage.md#diff_snapshots) | [`clef_sdk.storage`](clef/storage.md) | 计算两个 WorkspaceSnapshot 的变化 |
| [`digest_json`](clef/storage.md#digest_json) | [`clef_sdk.storage`](clef/storage.md) | 计算规范 JSON 的 SHA-256 |
| [`digest_path`](clef/verification.md#digest_path) | [`clef_sdk.verification`](clef/verification.md) | 计算文件或目录树 digest |
| [`domain_run`](clef/runtime.md#domain_run) | [`clef_sdk.runtime`](clef/runtime.md) | 执行一个 SessionTask |
| [`DomainRunner`](clef/runtime.md#domainrunner) | [`clef_sdk.runtime`](clef/runtime.md) | 执行 TaskRun attempt |
| [`DRAFT_2020_12_URI`](clef/verification.md#draft_2020_12_uri) | [`clef_sdk.verification`](clef/verification.md) | Draft 2020-12 dialect 的规范 URI |
| [`encode_report_envelope`](clef/protocol.md#encode_report_envelope) | [`clef_sdk.protocol`](clef/protocol.md) | 将 AgentReport 编码为 sentinel envelope |
| [`encode_report_json`](clef/protocol.md#encode_report_json) | [`clef_sdk.protocol`](clef/protocol.md) | 将 AgentReport 编码为规范 JSON |
| [`encode_request`](clef/protocol.md#encode_request) | [`clef_sdk.protocol`](clef/protocol.md) | 将 AgentRequest 编码为规范 JSON |
| [`execute_plan`](clef/runtime.md#execute_plan) | [`clef_sdk.runtime`](clef/runtime.md) | 编译并执行 WorkflowPlan |
| [`extract_report_envelope`](clef/protocol.md#extract_report_envelope) | [`clef_sdk.protocol`](clef/protocol.md) | 提取消息正文和 AgentReport |
| [`FakeAdapter`](clef/adapters.md#fakeadapter) | [`clef_sdk.adapters`](clef/adapters.md) | 执行预设响应或回调 |
| [`format_progress_event`](clef/runtime.md#format_progress_event) | [`clef_sdk.runtime`](clef/runtime.md) | 将 typed progress event 格式化为安全单行文本 |
| [`freeze_json`](clef/model.md#freeze_json) | [`clef_sdk.model`](clef/model.md) | 将 JSON 容器转换为 immutable 值 |
| [`JsonlTraceWriter`](clef/storage.md#jsonltracewriter) | [`clef_sdk.storage`](clef/storage.md) | 追加写入规范 JSONL trace |
| [`JsonSchemaDefinitionError`](clef/verification.md#jsonschemadefinitionerror) | [`clef_sdk.verification`](clef/verification.md) | 表示 Schema 非法或声明了其他 dialect |
| [`load_profile`](clef/profiles.md#load_profile) | [`clef_sdk.profiles`](clef/profiles.md) | 加载、解析和校验 TOML Profile |
| [`OpenCodeAdapter`](clef/adapters.md#opencodeadapter) | [`clef_sdk.adapters`](clef/adapters.md) | 通过 OpenCode CLI 执行 agent session |
| [`parse_task_result`](clef/model.md#parse_task_result) | [`clef_sdk.model`](clef/model.md) | 解析 TaskResultEnvelope 或包装旧任意 JSON |
| [`plan_digest`](clef/compiler.md#plan_digest) | [`clef_sdk.compiler`](clef/compiler.md) | 计算 WorkflowPlan 与 Profile 的联合 digest |
| [`profile_digest`](clef/profiles.md#profile_digest) | [`clef_sdk.profiles`](clef/profiles.md) | 计算脱敏 Profile digest |
| [`publish_once_bytes`](clef/storage.md#publish_once_bytes) | [`clef_sdk.storage`](clef/storage.md) | 原子发布一次字节内容 |
| [`resolve_profile_path`](clef/profiles.md#resolve_profile_path) | [`clef_sdk.profiles`](clef/profiles.md) | 解析 Profile 名称或文件路径 |
| [`snapshot_workspace`](clef/storage.md#snapshot_workspace) | [`clef_sdk.storage`](clef/storage.md) | 捕获 workspace 文件树状态 |
| [`sort_manifest_entries`](clef/storage.md#sort_manifest_entries) | [`clef_sdk.storage`](clef/storage.md) | 按 ArtifactOrderKey 排序 ManifestEntry |
| [`thaw_json`](clef/model.md#thaw_json) | [`clef_sdk.model`](clef/model.md) | 将 immutable JSON 值转换为普通容器 |
| [`uri_to_path`](clef/verification.md#uri_to_path) | [`clef_sdk.verification`](clef/verification.md) | 将本地 Artifact URI 转换为 Path |
| [`validate_json_schema`](clef/verification.md#validate_json_schema) | [`clef_sdk.verification`](clef/verification.md) | 按 JSON Schema Draft 2020-12 校验 JSON 值 |
| [`validate_plan`](clef/compiler.md#validate_plan) | [`clef_sdk.compiler`](clef/compiler.md) | 对 WorkflowPlan 执行静态检查 |
| [`VerifierRegistry`](clef/verification.md#verifierregistry) | [`clef_sdk.verification`](clef/verification.md) | 注册和执行 verifier |
| [`verify_outputs`](clef/verification.md#verify_outputs) | [`clef_sdk.verification`](clef/verification.md) | 执行 Artifact 约束和 verifier chain |
| [`visible_letter_number_count`](clef/verification.md#visible_letter_number_count) | [`clef_sdk.verification`](clef/verification.md) | 统计 Markdown 可见字母与数字 |
| [`WorkflowExecutor`](clef/runtime.md#workflowexecutor) | [`clef_sdk.runtime`](clef/runtime.md) | 调度静态 DAG |
| [`write_manifest`](clef/storage.md#write_manifest) | [`clef_sdk.storage`](clef/storage.md) | 写入 Artifact Manifest |
