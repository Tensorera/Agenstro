# `clef_sdk.storage` reference

本卷收录 10 个公共 API 和 24 个公共实体。所有符号均可从 `clef_sdk.storage` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`ArtifactOrderKey`](#artifactorderkey) | Entity | dataclass | Manifest 稳定排序键 |
| [`CacheConflictError`](#cacheconflicterror) | Entity | exception | 缓存内容冲突 |
| [`CacheCorruptionError`](#cachecorruptionerror) | Entity | exception | 缓存内容完整性错误 |
| [`CacheEligibility`](#cacheeligibility) | Entity | dataclass | 缓存写入资格 |
| [`CacheError`](#cacheerror) | Entity | exception | 缓存错误基类 |
| [`CacheHit`](#cachehit) | Entity | dataclass | 缓存命中结果 |
| [`CacheIdentity`](#cacheidentity) | Entity | dataclass | 缓存身份字段集合 |
| [`CacheNotEligibleError`](#cachenoteligibleerror) | Entity | exception | 缓存资格错误 |
| [`CASCorruptionError`](#cascorruptionerror) | Entity | exception | CAS 内容完整性错误 |
| [`CASError`](#caserror) | Entity | exception | CAS 错误基类 |
| [`CASObject`](#casobject) | Entity | dataclass | CAS 对象描述 |
| [`ChangeKind`](#changekind) | Entity | enum | WorkspaceDiff 变化类型 |
| [`ConservativeCache`](#conservativecache) | API | public API | 管理带资格检查的本地结果缓存 |
| [`ContentAddressedStore`](#contentaddressedstore) | API | public API | 按 SHA-256 保存和读取字节对象 |
| [`DeterministicManifestWriter`](#deterministicmanifestwriter) | API | public API | 写入确定性 Artifact Manifest |
| [`diff_snapshots`](#diff_snapshots) | API | public API | 计算两个 WorkspaceSnapshot 的变化 |
| [`digest_json`](#digest_json) | API | public API | 计算规范 JSON 的 SHA-256 |
| [`EntryKind`](#entrykind) | Entity | enum | SnapshotEntry 类型 |
| [`JsonlTraceWriter`](#jsonltracewriter) | API | public API | 追加写入规范 JSONL trace |
| [`ManifestConflictError`](#manifestconflicterror) | Entity | exception | Manifest 发布冲突 |
| [`ManifestEntry`](#manifestentry) | Entity | dataclass | Manifest Artifact 条目 |
| [`ManifestError`](#manifesterror) | Entity | exception | Manifest 错误基类 |
| [`ManifestWriteResult`](#manifestwriteresult) | Entity | dataclass | Manifest 写入结果 |
| [`publish_once_bytes`](#publish_once_bytes) | API | public API | 原子发布一次字节内容 |
| [`snapshot_workspace`](#snapshot_workspace) | API | public API | 捕获 workspace 文件树状态 |
| [`SnapshotEntry`](#snapshotentry) | Entity | dataclass | workspace 单路径快照 |
| [`SnapshotError`](#snapshoterror) | Entity | exception | workspace snapshot 错误 |
| [`sort_manifest_entries`](#sort_manifest_entries) | API | public API | 按 ArtifactOrderKey 排序 ManifestEntry |
| [`TraceError`](#traceerror) | Entity | exception | trace 写入错误 |
| [`TraceEvent`](#traceevent) | Entity | dataclass | 一条 trace 事件 |
| [`WorkspaceChange`](#workspacechange) | Entity | dataclass | workspace 单路径变化 |
| [`WorkspaceDiff`](#workspacediff) | Entity | dataclass | workspace 变化集合 |
| [`WorkspaceSnapshot`](#workspacesnapshot) | Entity | dataclass | workspace 文件树快照 |
| [`write_manifest`](#write_manifest) | API | public API | 写入 Artifact Manifest |

## ArtifactOrderKey

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ArtifactOrderKey`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ArtifactOrderKey
```

Namespace：`clef_sdk.storage`

输入字段：`stage`、`topo_rank`、`task_rank`、`output_rank`、`logical_name`。

输出：ManifestEntry 的稳定排序键。`as_dict()` 输出序列化字段，
`collision_key()` 输出大小写归一化碰撞键。

## CacheConflictError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheConflictError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheConflictError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：缓存键已有冻结内容时产生的 `CacheError`。

## CacheCorruptionError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheCorruptionError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheCorruptionError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：缓存 payload 或 Artifact digest 完整性检查产生的 `CacheError`。

## CacheEligibility

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheEligibility`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheEligibility
```

Namespace：`clef_sdk.storage`

输入字段：`declared_cacheable`、`succeeded`、`verified`、`effects_replayable`、
`has_undeclared_effects`。

输出：ConservativeCache 的写入资格。

## CacheError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：缓存错误基类实例。

## CacheHit

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheHit`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheHit
```

Namespace：`clef_sdk.storage`

输入字段：`key`、`payload`、`artifact_digests`、`created_at`。

输出：缓存查询结果。

## CacheIdentity

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheIdentity`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheIdentity
```

Namespace：`clef_sdk.storage`

输入字段：`domain_function`、`function_version`、`task_digest`、`input_digests`、
`prompt_digest`、`profile_digest`、`runtime_digest`、`verifier_digest`、
`protocol_version`。

输出：缓存 key 的完整身份。

## CacheNotEligibleError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CacheNotEligibleError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CacheNotEligibleError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：CacheEligibility 条件产生的 `CacheError`。

## CASCorruptionError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CASCorruptionError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CASCorruptionError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：CAS digest 复核产生的 `CASError`。

## CASError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CASError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CASError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：CAS 错误基类实例。

## CASObject

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.CASObject`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import CASObject
```

Namespace：`clef_sdk.storage`

输入字段：`digest`、`size`、`path`。

输出：ContentAddressedStore 中的对象描述。

## ChangeKind

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ChangeKind`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ChangeKind
```

Namespace：`clef_sdk.storage`

值：

```text
CREATED=created
DELETED=deleted
MODIFIED=modified
MOVED=moved
```

输出：WorkspaceChange 类型。

## ConservativeCache

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ConservativeCache`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ConservativeCache
```

构造：

```python
ConservativeCache(
    root: Path | str,
    *,
    cas: ContentAddressedStore | None = None,
    fsync: bool = False,
)
```

方法：

```python
lookup(identity: CacheIdentity) -> CacheHit | None
path_for(identity: CacheIdentity) -> Path
store(
    identity: CacheIdentity,
    payload: Any,
    *,
    eligibility: CacheEligibility,
    artifact_digests: tuple[str, ...] = (),
) -> CacheHit
```

输入：缓存根目录、`CacheIdentity`、JSON payload、资格状态和 Artifact digest。

输出：`CacheHit`、缓存文件路径或 `None`。

## ContentAddressedStore

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ContentAddressedStore`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ContentAddressedStore
```

构造：

```python
ContentAddressedStore(
    root: Path | str,
    *,
    fsync: bool = False,
)
```

方法：

```python
get_bytes(digest: str, *, max_bytes: int | None = None) -> bytes
has(digest: str, *, verify: bool = False) -> bool
materialize(
    digest: str,
    destination: Path | str,
    *,
    overwrite: bool = False,
) -> Path
open(digest: str) -> BinaryIO
path_for(digest: str) -> Path
put_bytes(data: bytes | bytearray | memoryview) -> CASObject
put_file(source: Path | str, *, chunk_size: int = 1048576) -> CASObject
put_text(text: str, *, encoding: str = "utf-8") -> CASObject
verify(digest: str) -> CASObject
```

输入：字节、文本、文件路径、SHA-256 digest 和物化路径。

输出：`CASObject`、字节流、二进制文件句柄、路径或存在状态。

## DeterministicManifestWriter

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.DeterministicManifestWriter`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import DeterministicManifestWriter
```

构造：

```python
DeterministicManifestWriter(
    path: Path | str,
    *,
    fsync: bool = False,
)
```

方法：

```python
write(
    entries: Iterable[ManifestEntry],
    *,
    run_id: str | None = None,
    plan_digest: str | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> ManifestWriteResult
```

输入：目标路径、ManifestEntry 集合、run identity、plan digest 和 metadata。

输出：`ManifestWriteResult`。
## diff_snapshots

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.diff_snapshots`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import diff_snapshots
```

```python
diff_snapshots(
    before: WorkspaceSnapshot,
    after: WorkspaceSnapshot,
    *,
    detect_moves: bool = True,
) -> WorkspaceDiff
```

输入：两个 WorkspaceSnapshot 和 move 检测开关。

输出：包含 created、modified、deleted 和 moved 的 `WorkspaceDiff`。

## digest_json

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.digest_json`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import digest_json
```

```python
digest_json(value: Any) -> str
```

输入：JSON 兼容值。

输出：规范 JSON 字节的 64 位小写 SHA-256。

## EntryKind

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.EntryKind`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import EntryKind
```

Namespace：`clef_sdk.storage`

值：

```text
DIRECTORY=directory
FILE=file
SYMLINK=symlink
```

输出：SnapshotEntry 的文件系统类型。

## JsonlTraceWriter

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.JsonlTraceWriter`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import JsonlTraceWriter
```

构造：

```python
JsonlTraceWriter(
    path: Path | str,
    *,
    run_id: str,
    fsync: bool = False,
    clock: Callable[[], datetime] = ...,
    observer: Callable[[TraceEvent], None] | None = None,
)
```

执行：

```python
emit(
    event: str,
    data: Mapping[str, Any] | None = None,
    *,
    task_id: str | None = None,
    level: str = "info",
) -> TraceEvent
```

输入：trace 路径、run identity、事件名称、事件数据、task identity、level 和
可选底层 trace observer。observer 只在对应 JSONL record 已经追加成功后、path
lock 之外调用；observer 异常被忽略。

输出：写入完成的 `TraceEvent`。

## ManifestConflictError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ManifestConflictError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ManifestConflictError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：冻结 Manifest 内容冲突产生的 `ManifestError`。

## ManifestEntry

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ManifestEntry`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ManifestEntry
```

Namespace：`clef_sdk.storage`

输入字段：`artifact_id`、`order_key`、`uri`、`kind`、`description`、`sha256`、
`size`、`media_type`、`producer_task_id`。

输出：Artifact Manifest 中的一条记录。`as_dict(ordinal=...)` 输出序列化字段。

## ManifestError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ManifestError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ManifestError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：Manifest 错误基类实例。

## ManifestWriteResult

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.ManifestWriteResult`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import ManifestWriteResult
```

Namespace：`clef_sdk.storage`

输入字段：`path`、`digest`、`entry_count`、`changed`。

输出：Manifest 写入结果。

## publish_once_bytes

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.publish_once_bytes`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import publish_once_bytes
```

```python
publish_once_bytes(
    path: Path,
    data: bytes,
    *,
    fsync: bool = False,
) -> bool
```

输入：目标路径、字节内容和 fsync 开关。

输出：本次调用完成目标创建时为 `True`，目标已经存在时为 `False`。

## snapshot_workspace

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.snapshot_workspace`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import snapshot_workspace
```

```python
snapshot_workspace(
    root: Path | str,
    *,
    exclude_paths: Iterable[Path | str] = (),
    include_directories: bool = True,
) -> WorkspaceSnapshot
```

输入：workspace 根目录、排除路径和目录记录开关。

输出：`WorkspaceSnapshot`。

## SnapshotEntry

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.SnapshotEntry`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import SnapshotEntry
```

Namespace：`clef_sdk.storage`

输入字段：`path`、`kind`、`size`、`mtime_ns`、`mode`、`sha256`、
`link_target`。

输出：WorkspaceSnapshot 中的一个文件系统条目。

## SnapshotError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.SnapshotError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import SnapshotError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：workspace snapshot 和 diff 处理产生的异常。

## sort_manifest_entries

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.sort_manifest_entries`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import sort_manifest_entries
```

```python
sort_manifest_entries(
    entries: Iterable[ManifestEntry],
) -> tuple[ManifestEntry, ...]
```

输入：ManifestEntry iterable。

输出：按 ArtifactOrderKey 排序的 tuple。

## TraceError

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.TraceError`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import TraceError
```

Namespace：`clef_sdk.storage`

输入：异常消息。

输出：trace 事件校验和写入产生的异常。

## TraceEvent

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.TraceEvent`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import TraceEvent
```

Namespace：`clef_sdk.storage`

输入字段：`schema_version`、`sequence`、`timestamp`、`run_id`、`event`、
`level`、`task_id`、`data`。

输出：JSONL trace 中的一条事件。

## WorkspaceChange

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.WorkspaceChange`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import WorkspaceChange
```

Namespace：`clef_sdk.storage`

输入字段：`kind`、`path`、`old_path`、`before`、`after`。

输出：WorkspaceDiff 中的一条变化。

## WorkspaceDiff

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.WorkspaceDiff`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import WorkspaceDiff
```

Namespace：`clef_sdk.storage`

输入字段：`root`、`created`、`modified`、`deleted`、`moved`。

输出：两个 WorkspaceSnapshot 之间的变化集合。`changes` 属性提供稳定总序列。

## WorkspaceSnapshot

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.WorkspaceSnapshot`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import WorkspaceSnapshot
```

Namespace：`clef_sdk.storage`

输入字段：`root`、`entries`、`captured_at_ns`。

输出：workspace 文件树的 immutable 快照。`by_path()` 输出 path 到
SnapshotEntry 的 mapping。
## write_manifest

**Canonical FQN（规范完全限定名）**：`clef_sdk.storage.write_manifest`

**Canonical import（规范导入）**：

```python
from clef_sdk.storage import write_manifest
```

```python
write_manifest(
    path: Path | str,
    entries: Iterable[ManifestEntry],
    *,
    run_id: str | None = None,
    plan_digest: str | None = None,
    metadata: Mapping[str, Any] | None = None,
    fsync: bool = False,
) -> ManifestWriteResult
```

输入：目标路径、ManifestEntry 集合、run identity、plan digest、metadata 和 fsync
开关。

输出：`ManifestWriteResult`。
