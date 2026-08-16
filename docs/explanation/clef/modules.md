# Clef SDK 模块

本页解释包之间的职责和直接依赖方向。精确的公开名称和签名以
[API 参考](../../reference/clef-api.md) 与
[实体参考](../../reference/clef-entities.md) 为准。

## 1. 模块清单

| 模块 | 职责 | 主要入口 |
| --- | --- | --- |
| `adapters` | agent transport | `AgentAdapter`, `OpenCodeAdapter`, `FakeAdapter` |
| `compiler` | 计划规范化、静态检查、digest | `compile_plan`, `validate_plan` |
| `model` | immutable entities、枚举、JSON | `SessionTask`, `WorkflowPlan`, `JsonModel` |
| `profiles` | TOML 配置、路径解析、配置 digest | `load_profile`, `Profile` |
| `protocol` | AgentRequest、AgentReport、codec | `encode_request`, `decode_report_envelope` |
| `runtime` | TaskRun、DAG 调度、retry | `DomainRunner`, `WorkflowExecutor` |
| `storage` | snapshot、CAS、trace、cache、Manifest | `ContentAddressedStore`, `snapshot_workspace` |
| `verification` | verifier registry 和内建验证 | `VerifierRegistry`, `verify_outputs` |

## 2. 依赖路线

下表只列跨包的直接源码依赖。“使用方”导入“依赖”，方向不表示控制权或
运行时回调方向。

| 使用方 | 直接依赖 |
| --- | --- |
| `protocol` | `model` |
| `verification` | `model` |
| `compiler` | `model`, `profiles`, `verification.json_schema` |
| `runtime` | `adapters`, `compiler`, `model`, `profiles`, `protocol`, `storage`, `verification` |

`adapters`、`profiles` 和 `storage` 不依赖 `runtime`；`runtime` 才是组装
compiler、protocol、adapter、verification 和 storage 的上层。

## 3. `adapters`

输入：

- prompt；
- workspace；
- title；
- session identity。

输出：`AdapterExecution`。

`OpenCodeAdapter` 负责进程参数、stdin、自然等待 session idle、原生 compact、
事件解析和 usage 观测；它不设置 wall-clock timeout。
`FakeAdapter` 负责预设响应和回调执行。

## 4. `compiler`

输入：WorkflowPlan 和 Profile。

输出：CompiledPlan 或 PlanValidationReport。

compiler 负责：

- task 规范化；
- prompt 排序；
- output path 解析；
- contract 绑定；
- binding 检查；
- DAG 检查；
- plan digest。

## 5. `model`

model 提供：

- Artifact entities；
- Contract entities；
- Prompt entities；
- Result entities；
- Task entities；
- Workflow entities；
- enums；
- canonical JSON；
- FrozenDict。

所有核心 entity 使用 frozen dataclass 和 slots。

## 6. `profiles`

profiles 负责：

- TOML 解析；
- unknown key 检查；
- 相对路径解析；
- workspace 与 state root 检查；
- adapter 配置；
- runtime 配置；
- storage 配置；
- 配置脱敏；
- digest。

## 7. `protocol`

protocol 负责：

- AgentRequest；
- AgentReport；
- JSON codec；
- sentinel envelope；
- protocol version；
- request/report correlation；
- protocol errors。

## 8. `runtime`

runtime 包含：

- `DomainRunner`；
- `runtime/taskrun/` session recovery 状态机；
- `WorkflowExecutor`；
- retry feedback；
- session identity registry；
- workspace 生命周期；
- typed progress observer、legacy callback 和 console formatter；
- task/workflow run summary；
- workflow trace。

## 9. `storage`

storage 包含：

- WorkspaceSnapshot；
- WorkspaceDiff；
- ContentAddressedStore；
- JsonlTraceWriter；
- ConservativeCache；
- ArtifactOrderKey；
- Manifest writer；
- publish-once。

## 10. `verification`

verification 包含：

- Verifier callable；
- VerificationContext；
- VerifierRegistry；
- ArtifactConstraint 转换；
- built-in verifiers；
- 基于 `python-jsonschema` 的 Draft 2020-12 校验边界；
- VerificationReport 聚合。

调用方通过 registry 注册领域 verifier。
