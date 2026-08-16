# Profile boundaries

## 1. Profile 结构

```toml
name = "profile-name"

[adapter]

[runtime]

[workspace]

[storage]

[labels]
```

`load_profile()` 将 TOML 转换为 immutable Profile。

## 2. 路径解析

Profile 文件路径先转换为绝对 Path。TOML 中的 workspace、read root 和 state root
以 Profile 文件目录为解析基准。

```text
profile.toml parent
  + relative configured path
  -> resolve
  -> absolute Path
```

`resolve_profile_path()` 支持 Profile 名称和显式 `.toml` 路径。

## 3. AdapterConfig

字段组：

- executable；
- model；
- agent；
- output format；
- variant；
- attach URL；
- pure；
- auto approve；
- environment inheritance；
- required environment names；
- extra arguments。

Profile 保存环境变量名称。`redacted_dict()` 生成持久化配置。
`auto_approve` 默认 `true`，因此 adapter 向 `opencode run` 传递 `--auto`。
OpenCode 配置中的显式 `deny` 不会被覆盖。

## 4. RuntimeConfig

字段组：

- max concurrency；
- max attempts；
- retry backoff；
- fail fast；
- max subagent depth；
- max fan-out；
- session reuse。

`session_reuse` 的协议 1.0 取值为 `never`。

## 5. WorkspaceConfig

字段组：

- root；
- read roots。

`resolve_write_path()` 将候选路径解析到 workspace root。
`resolve_read_path()` 将候选路径解析到 effective read roots。

WorkspaceConfig 只定义 Clef 的 Artifact 路径解析边界，不是工具权限模型。
shell、network、edit、delete、move、external directory 等权限直接写入 OpenCode
的全局或项目级 `opencode.json`。Clef 不读取、合并或二次执行这些规则。
旧 Profile 中的 `allowed_shells`、`network`、`network_allowlist`、
`allow_delete`、`allow_move` 应迁移到 OpenCode；loader 会把这些旧字段视为未知
字段，防止配置看似生效但实际上被忽略。

Motivo Studio 不读取这个 Clef Profile。Studio 只把 workflow occurrence
状态交给 Codex/OpenCode CLI；模型、provider、agent、凭据、工具和 permission
继续由各 CLI 的用户级或项目级配置管理。

## 6. StorageConfig

字段组：

- state root；
- CAS directory；
- traces directory；
- cache directory；
- manifests directory；
- cache enabled；
- fsync。

派生属性：

```text
cas_root
traces_root
cache_root
manifests_root
```

state root 与 workspace root 形成独立目录树。

## 7. Labels

labels 为字符串 key/value tuple。Profile 构造时执行 key 排序。Profile digest
覆盖脱敏 labels。

## 8. Filesystem validation

`Profile.validate_filesystem()` 检查：

- workspace root；
- effective read roots；
- state root 类型；
- 路径解析结果；
- 目录边界。

## 9. Profile digest

digest 流程：

```text
Profile
  -> redacted_dict
  -> sorted JSON keys
  -> UTF-8
  -> SHA-256
```

输出为 64 位小写十六进制字符串。

## 10. Runtime injection

```text
Profile
  -> compile_plan
  -> DomainRunner.from_profile
  -> WorkflowExecutor.from_profile
  -> adapter configuration
  -> storage roots
  -> retry and concurrency policy
  -> Artifact workspace paths
```

同一 Profile 实例贯穿编译、执行和运行记录。
