# Artifact 排序

本页解释调用方如何使用 ArtifactOrderKey 和 Manifest writer 获得稳定输出顺序。
精确构造签名与字段以 [API 参考](../../reference/clef-api.md) 和
[实体参考](../../reference/clef-entities.md) 为准。

Clef SDK 提供排序、碰撞检查和 publish-once 原语，但不会自动从
WorkflowPlan 派生 ArtifactOrderKey、artifact ID 或 ManifestEntry。

## 1. 目标

调用方可以从 WorkflowPlan 的稳定字段生成 Artifact 排序信息。并发调度、
session、attempt 和时间字段应只留在运行记录，不应进入稳定排序键。

## 2. ArtifactOrderKey

```python
ArtifactOrderKey(
    stage: int | str,
    topo_rank: int,
    task_rank: int,
    output_rank: int,
    logical_name: str,
)
```

字段：

| 字段 | 含义 |
| --- | --- |
| `stage` | 调用方定义的逻辑阶段 |
| `topo_rank` | task 拓扑位置 |
| `task_rank` | 同层 task 稳定序号 |
| `output_rank` | task output 稳定序号 |
| `logical_name` | Artifact 逻辑名称 |

## 3. 排序元组

```text
(
  normalized_stage,
  topo_rank,
  task_rank,
  output_rank,
  logical_name.casefold(),
  logical_name
)
```

数字 stage 使用 `(0, stage)`。字符串 stage 使用
`(1, (stage.casefold(), stage))`。

## 4. 文本规范化

排序文本执行：

- 首尾空白清理；
- Unicode NFC；
- 非空校验；
- NUL 校验。

`collision_key()` 使用大小写归一化 logical name。

## 5. Artifact identity

框架没有内建 artifact ID 派生函数。调用方定义稳定 artifact ID 时，建议覆盖：

```text
plan identity
producer task ID
declared output name
ordering fields
```

内容字段：

```text
SHA-256
size
media type
URI
```

## 6. ManifestEntry

```python
ManifestEntry(
    artifact_id=...,
    order_key=...,
    uri=...,
    kind=...,
    description=...,
    sha256=...,
    size=...,
    media_type=...,
    producer_task_id=...,
)
```

Manifest writer 为排序后的 entry 分配从 1 开始的 ordinal。

## 7. Manifest document

```text
schema_version
run_id
plan_digest
artifacts
metadata
```

序列化使用 canonical JSON 和末尾换行。Manifest digest 为完整文档字节的
SHA-256。

## 8. Publish-once

写入流程：

```text
ManifestEntry iterable
  -> validate
  -> sort
  -> canonical JSON
  -> digest
  -> publish_once_bytes
  -> ManifestWriteResult
```

首次发布产生 `changed=True`。幂等调用产生 `changed=False`。内容冲突生成
`ManifestConflictError`。

## 9. Artifact 发布路线

TaskRun 自动发布到已经验证的 ArtifactRef，并在适用时写入文件 CAS。Manifest 是
之后的显式调用方步骤：

```text
attempt output
  -> verifier chain
  -> digest recheck
  -> verified ArtifactRef
  -> caller mapping
  -> ManifestEntry
  -> DeterministicManifestWriter
```

调用方应只从 verified ArtifactRef 生成 completed entry；Manifest writer 只验证
entry 结构、排序和发布冲突，不会判断 Artifact 是否来自成功的 TaskRun。

## 10. 调用方映射

调用方负责定义：

- stage 编号；
- task rank；
- output rank；
- logical name；
- stable output URI；
- artifact ID 派生函数；
- ordering version。

Clef SDK 负责排序、碰撞检查、序列化和 publish-once。
