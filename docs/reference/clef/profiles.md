# `clef_sdk.profiles` reference

本卷收录 4 个公共 API 和 9 个公共实体。所有符号均可从 `clef_sdk.profiles` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`AdapterConfig`](#adapterconfig) | Entity | dataclass | OpenCode adapter 配置 |
| [`default_profiles_dir`](#default_profiles_dir) | API | public API | 计算平台默认 Profile 目录 |
| [`load_profile`](#load_profile) | API | public API | 加载、解析和校验 TOML Profile |
| [`ModelRoute`](#modelroute) | Entity | dataclass | logical effort 对应的 OpenCode 模型路由 |
| [`Profile`](#profile) | Entity | dataclass | 完整运行配置 |
| [`profile_digest`](#profile_digest) | API | public API | 计算脱敏 Profile digest |
| [`ProfileError`](#profileerror) | Entity | exception | Profile 错误基类 |
| [`ProfilePathError`](#profilepatherror) | Entity | exception | Profile 路径错误 |
| [`ProfileValidationError`](#profilevalidationerror) | Entity | exception | Profile 字段校验错误 |
| [`resolve_profile_path`](#resolve_profile_path) | API | public API | 解析 Profile 名称或文件路径 |
| [`RuntimeConfig`](#runtimeconfig) | Entity | dataclass | scheduler 配置 |
| [`StorageConfig`](#storageconfig) | Entity | dataclass | 框架状态目录配置 |
| [`WorkspaceConfig`](#workspaceconfig) | Entity | dataclass | workspace 能力配置 |

## AdapterConfig

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.AdapterConfig`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import AdapterConfig
```

Namespace：`clef_sdk.profiles`

输入字段：

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `executable` | `str` | `"opencode"` | CLI 名称或路径 |
| `model` | `str \| None` | `None` | 模型标识 |
| `agent` | `str` | `"build"` | agent 名称 |
| `output_format` | `str` | `"json"` | CLI 输出格式 |
| `variant` | `str \| None` | `None` | 模型 variant |
| `attach_url` | `str \| None` | `None` | OpenCode 服务 URL |
| `pure` | `bool` | `False` | pure 模式 |
| `auto_approve` | `bool` | `True` | OpenCode `--auto` 配置 |
| `inherit_environment` | `bool` | `True` | 环境继承配置 |
| `required_env` | `tuple[str, ...]` | `()` | 必需环境变量名称 |
| `extra_args` | `tuple[str, ...]` | `()` | CLI 附加参数 |
| `models` | `Mapping[str \| Effort, ModelRoute]` | empty | `xhigh/high/medium/low` 逻辑路由；规范化为 `FrozenDict` |

输出：immutable adapter 配置。`redacted_dict()` 输出脱敏 dict。空 `models`
不会写入该 dict，因此旧 Profile digest 保持不变。

## ModelRoute

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.ModelRoute`

```python
from clef_sdk import ModelRoute
```

字段：`model: str`、`variant: str | None = None`。`model` 是
`opencode models` 返回的完整模型标识；`variant` 是该模型的 OpenCode/provider
原生 variant，与 Clef 的 logical effort 无关。

规范 TOML 写法：

```toml
[adapter.models.xhigh]
model = "openai/gpt-5.6-sol"
variant = "low"

[adapter.models.low]
model = "anthropic/claude-4.5-sonnet"
variant = "xhigh"
```

不需要 variant 时也支持简写：

```toml
[adapter.models]
medium = "provider/model-id"
```

## default_profiles_dir

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.default_profiles_dir`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import default_profiles_dir
```

```python
default_profiles_dir(
    environ: Mapping[str, str] | None = None,
) -> Path
```

输入：可选环境变量 mapping。

输出：平台 Profile 目录的绝对 Path。函数执行路径计算。

## load_profile

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.load_profile`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import load_profile
```

```python
load_profile(
    reference: str | os.PathLike[str],
    *,
    profiles_dir: Path | str | None = None,
    base_dir: Path | str | None = None,
    require_workspace: bool = True,
    require_read_roots: bool = True,
) -> Profile
```

输入：

- `reference`：Profile 名称或 TOML 路径；
- `profiles_dir`：名称解析目录；
- `base_dir`：相对文件路径解析目录；
- `require_workspace`：workspace 文件系统检查开关；
- `require_read_roots`：read root 文件系统检查开关。

输出：规范化 `Profile`。

## Profile

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.Profile`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import Profile
```

Namespace：`clef_sdk.profiles`

输入字段：`name`、`adapter`、`runtime`、`workspace`、`storage`、`labels`、
`source_path`。

输出：完整 immutable 运行配置。

公开成员：

```python
digest -> str
missing_required_environment(environ=None) -> tuple[str, ...]
redacted_dict() -> dict[str, object]
validate_filesystem(
    *,
    require_workspace: bool = True,
    require_read_roots: bool = True,
) -> None
```

## profile_digest

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.profile_digest`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import profile_digest
```

```python
profile_digest(profile: Profile) -> str
```

输入：Profile。

输出：脱敏规范配置的 64 位小写 SHA-256。

## ProfileError

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.ProfileError`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import ProfileError
```

Namespace：`clef_sdk.profiles`

输入：异常消息。

输出：Profile 错误基类实例。

## ProfilePathError

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.ProfilePathError`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import ProfilePathError
```

Namespace：`clef_sdk.profiles`

输入：异常消息。

输出：Profile 路径解析和边界检查产生的 `ProfileError`。

## ProfileValidationError

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.ProfileValidationError`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import ProfileValidationError
```

Namespace：`clef_sdk.profiles`

输入：异常消息。

输出：Profile schema 校验产生的 `ProfileError`。

## resolve_profile_path

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.resolve_profile_path`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import resolve_profile_path
```

```python
resolve_profile_path(
    reference: str | os.PathLike[str],
    *,
    profiles_dir: Path | str | None = None,
    base_dir: Path | str | None = None,
    must_exist: bool = True,
) -> Path
```

输入：Profile 名称或路径、解析目录和存在检查开关。

输出：扩展名为 `.toml` 的绝对 Path。

## RuntimeConfig

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.RuntimeConfig`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import RuntimeConfig
```

Namespace：`clef_sdk.profiles`

输入字段：

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `max_concurrency` | `int` | `1` | scheduler 并发上限 |
| `max_attempts` | `int` | `1` | runtime attempt 上限 |
| `retry_backoff_seconds` | `float` | `0.0` | retry 间隔 |
| `fail_fast` | `bool` | `True` | workflow 快速结束配置 |
| `max_subagent_depth` | `int` | `1` | 保留字段；当前不执行 depth enforcement |
| `max_fan_out` | `int` | `32` | fan-out 上限 |
| `session_reuse` | `str` | `"never"` | session 策略 |

输出：Profile 的 scheduler 配置。`as_dict()` 输出配置 dict。

`max_subagent_depth` 会被校验和序列化，但当前 compiler/runtime 不计算
subagent depth。

## StorageConfig

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.StorageConfig`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import StorageConfig
```

Namespace：`clef_sdk.profiles`

输入字段：

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `state_root` | `Path` | 必填 | 状态根目录 |
| `cas_dir` | `str` | `"cas"` | CAS 子目录 |
| `traces_dir` | `str` | `"traces"` | trace 子目录 |
| `cache_dir` | `str` | `"cache"` | 调用方 cache 子目录 |
| `manifests_dir` | `str` | `"manifests"` | 调用方 Manifest 子目录 |
| `cache_enabled` | `bool` | `True` | 调用方 cache 偏好；runtime 当前不读取 |
| `fsync` | `bool` | `False` | fsync 开关 |

输出：Profile 的存储配置。

属性：`cas_root`、`traces_root`、`cache_root`、`manifests_root`。

## WorkspaceConfig

**Canonical FQN（规范完全限定名）**：`clef_sdk.profiles.WorkspaceConfig`

**Canonical import（规范导入）**：

```python
from clef_sdk.profiles import WorkspaceConfig
```

Namespace：`clef_sdk.profiles`

输入字段：

| 字段 | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `root` | `Path` | 必填 | 可写 workspace 根目录 |
| `read_roots` | `tuple[Path, ...]` | `()` | 读取根目录 |

输出：Profile 的 Artifact 路径配置，不包含 OpenCode permission。

公开成员：

```python
effective_read_roots -> tuple[Path, ...]
resolve_read_path(value: Path | str) -> Path
resolve_write_path(value: Path | str) -> Path
as_dict() -> dict[str, object]
```
