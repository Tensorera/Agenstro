# 验证与存储

本页解释验证流水线以及 runtime 使用或公开的本地存储原语。精确 verifier、
存储 API 与实体字段以 [API 参考](../../reference/clef-api.md)
和 [实体参考](../../reference/clef-entities.md) 为准。

TaskRun 自动使用 snapshot、trace 和文件 CAS；cache 与 Manifest 是调用方拥有的
可选原语，`DomainRunner` 和 `WorkflowExecutor` 不会自动读写它们。

## 1. Verification pipeline

```text
SessionTask.outputs
  -> implicit ArtifactConstraint verifiers
  -> DomainContract.verifiers
  -> CheckResult sequence
  -> VerificationReport
```

`verify_outputs()` 创建 VerificationContext：

```python
VerificationContext(
    task=task,
    workspace=workspace,
    outputs=outputs,
)
```

## 2. VerifierRegistry

registry 保存：

```text
stable verifier name -> Verifier callable
```

公开操作：

- `register()`；
- `get()`；
- `names()`；
- `run()`。

Verifier callable 签名：

```python
def verifier(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult: ...
```

## 3. Built-in verifiers

| 名称 | 输入 | 输出 |
| --- | --- | --- |
| `command_exit` | command、exit code | CheckResult |
| `digest` | output、expected digest | CheckResult |
| `file_exists` | output | CheckResult |
| `json_schema` | output、Draft 2020-12 schema | CheckResult |
| `markdown_images` | output、minimum | CheckResult |
| `media_type` | output、expected | CheckResult |
| `min_text_length` | output、minimum | CheckResult |
| `size` | output、minimum、maximum | CheckResult |

`command_exit` 不接受 `timeout_seconds`。被调用脚本必须自行实现 deadline，并以
非零退出码和 stderr 报告超时。

默认 registry 每次调用生成新实例。调用方在实例上注册领域 verifier。

### 3.1 JSON Schema 边界

`json_schema` verifier 的标准化路线如下：

```text
VerifierSpec.parameters.schema
  -> immutable JSON thaw
  -> Draft 2020-12 meta-schema check
  -> strict Artifact JSON decode
  -> Draft202012Validator.iter_errors
  -> deterministic JSON Pointer errors
```

Schema object 和 boolean schema 都是合法入口。未写 `$schema` 时框架将 dialect
固定为 Draft 2020-12；显式声明其他 dialect 会产生 verifier error。外部资源检索
保持关闭，本地 `$defs`、anchor、compound schema 和同文档 `$ref` 正常解析。
`format` 保持 Draft 2020-12 默认的 annotation 行为。正则关键字采用
`python-jsonschema` 的 Python regular-expression 语义。

`validate_plan()` 会对 Artifact JSON Schema constraint 和显式 `json_schema`
verifier 执行同一 meta-schema 检查。非法 Schema 在调度 agent 前形成
`invalid_json_schema` PlanIssue；运行时复查覆盖直接 verifier 调用。

## 4. VerificationReport

聚合字段：

- `passed`；
- `checks`；
- `score`；
- `evidence`。

score 来源为 CheckResult.score 的平均值。check 集合提供完整验证证据。

## 5. Workspace snapshot

`snapshot_workspace()` 捕获：

- relative path；
- EntryKind；
- size；
- mtime；
- mode；
- SHA-256；
- link target。

目录遍历使用稳定路径排序。WorkspaceSnapshot 通过 `by_path()` 建立查询 mapping。

## 6. Workspace diff

`diff_snapshots()` 输出：

- created；
- modified；
- deleted；
- moved。

WorkspaceChange 保存 before 和 after SnapshotEntry。

## 7. Content Addressed Store

CAS key：

```text
sha256(file bytes)
```

存储路线：

```text
source bytes
  -> SHA-256
  -> temporary file
  -> atomic publish
  -> CASObject
```

读取路线：

```text
digest
  -> path_for
  -> verify
  -> open/get_bytes/materialize
```

CASObject 包含 digest、size 和 path。

## 8. Trace

JsonlTraceWriter 为单个 run 维护单调 sequence。

TraceEvent 字段：

- schema version；
- sequence；
- timestamp；
- run ID；
- event；
- level；
- task ID；
- data。

每个事件编码为一行 canonical JSON。

## 9. Cache

CacheIdentity 覆盖：

- domain function；
- function version；
- task digest；
- input digests；
- prompt digest；
- Profile digest；
- runtime digest；
- verifier digest；
- protocol version。

CacheEligibility 覆盖：

- cacheable 声明；
- succeeded；
- verified；
- effect replayability；
- undeclared effect 状态。

ConservativeCache 将 payload 和 Artifact digest 写入稳定 key 路径。

当前 runtime 不构造 `ConservativeCache`，也不读取
`StorageConfig.cache_enabled` 来执行自动 lookup/store。调用方必须显式创建
cache、构造 CacheIdentity/CacheEligibility，并决定何时查询或写入。

## 10. Manifest

ManifestEntry 包含：

- artifact ID；
- ArtifactOrderKey；
- URI；
- kind；
- description；
- SHA-256；
- size；
- media type；
- producer task ID。

DeterministicManifestWriter 先排序 entry，再生成 canonical JSON 和 payload digest。

当前 runtime 不构造 ManifestEntry，也不向 `StorageConfig.manifests_root` 自动
写入 Manifest。调用方负责从已经验证的 SessionResult 或 WorkflowResult 输出
生成 entry 并调用 writer。

## 11. 发布顺序

TaskRun 的自动发布路线是：

```text
workspace output
  -> ArtifactRef resolution
  -> effect audit
  -> verifier chain
  -> digest stability check
  -> CAS
  -> SessionResult
```

文件 Artifact 进入 CAS；目录 Artifact 当前跳过 CAS。使用
`RetryWorkspaceStrategy.NEW` 时，scheduler 还会把私有 attempt 中已验证的输出
发布到 task 的稳定 output slot。

可选 Manifest 路线由调用方显式启动：

```text
verified SessionResult/WorkflowResult outputs
  -> caller-defined artifact ID and ArtifactOrderKey
  -> ManifestEntry
  -> DeterministicManifestWriter
```

trace 记录 runtime 自动执行阶段的状态和 identity，不记录调用方之后未通过
runtime 发起的 cache 或 Manifest 操作。
