# `clef_sdk.verification` reference

本卷收录 10 个公共 API 和 2 个公共实体。所有符号均可从 `clef_sdk.verification` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`check_json_schema`](#check_json_schema) | API | public API | 用 Draft 2020-12 meta-schema 检查 Schema |
| [`default_registry`](#default_registry) | API | public API | 创建通用 verifier registry |
| [`digest_path`](#digest_path) | API | public API | 计算文件或目录树 digest |
| [`DRAFT_2020_12_URI`](#draft_2020_12_uri) | API | public API | Draft 2020-12 dialect 的规范 URI |
| [`JsonSchemaDefinitionError`](#jsonschemadefinitionerror) | API | public API | 表示 Schema 非法或声明了其他 dialect |
| [`uri_to_path`](#uri_to_path) | API | public API | 将本地 Artifact URI 转换为 Path |
| [`validate_json_schema`](#validate_json_schema) | API | public API | 按 JSON Schema Draft 2020-12 校验 JSON 值 |
| [`VerificationContext`](#verificationcontext) | Entity | dataclass | verifier 可信上下文 |
| [`Verifier`](#verifier) | Entity | type alias | verifier callable 类型 |
| [`VerifierRegistry`](#verifierregistry) | API | public API | 注册和执行 verifier |
| [`verify_outputs`](#verify_outputs) | API | public API | 执行 Artifact 约束和 verifier chain |
| [`visible_letter_number_count`](#visible_letter_number_count) | API | public API | 统计 Markdown 可见字母与数字 |

## check_json_schema

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.check_json_schema`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import check_json_schema
```

```python
check_json_schema(
    schema: Mapping[str, Any] | bool,
) -> None
```

输入：JSON Schema object 或 boolean schema。

输出：无。函数使用 `Draft202012Validator.check_schema()` 对 Schema 本身执行
Draft 2020-12 meta-schema 校验。Schema 未声明 `$schema` 时按 Draft 2020-12
解释；声明时必须使用 `DRAFT_2020_12_URI`，否则抛出
`JsonSchemaDefinitionError`。

## default_registry

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.default_registry`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import default_registry
```

```python
default_registry() -> VerifierRegistry
```

输入：无。

输出：包含 `command_exit`、`digest`、`file_exists`、`json_schema`、
`markdown_images`、`media_type`、`min_text_length` 和 `size` 的新 registry。

## digest_path

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.digest_path`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import digest_path
```

```python
digest_path(path: Path) -> str
```

输入：文件路径或目录路径。

输出：`sha256:<64 lowercase hex>`。目录 digest 覆盖稳定排序后的目录树。

## DRAFT_2020_12_URI

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.DRAFT_2020_12_URI`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import DRAFT_2020_12_URI
```

```python
DRAFT_2020_12_URI = "https://json-schema.org/draft/2020-12/schema"
```

输出：框架接受的 JSON Schema dialect URI。

## JsonSchemaDefinitionError

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.JsonSchemaDefinitionError`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import JsonSchemaDefinitionError
```

```python
class JsonSchemaDefinitionError(ValueError)
```

输出：Schema 未通过 Draft 2020-12 meta-schema 校验，或显式声明了其他
dialect 时抛出的异常。

## uri_to_path

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.uri_to_path`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import uri_to_path
```

```python
uri_to_path(uri: str) -> Path
```

输入：本地路径字符串、Windows 绝对路径、UNC 路径或 `file:` URI。

输出：Path。

## validate_json_schema

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.validate_json_schema`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import validate_json_schema
```

```python
validate_json_schema(
    value: Any,
    schema: Mapping[str, Any] | bool,
) -> list[str]
```

输入：JSON 值和 Draft 2020-12 Schema。

输出：按 instance JSON Pointer 稳定排序的验证消息列表；空列表表示通过。Schema
非法时抛出 `JsonSchemaDefinitionError`，无法解析的引用抛出引用解析错误。

实现语义：

- Schema object 与 boolean schema 均受支持；
- Schema 本身先通过官方 Draft 2020-12 meta-schema 检查；
- `type` union、组合关键字、条件关键字、`$defs`、本地 `$ref`、
  `prefixItems`、`unevaluated*` 等由 `Draft202012Validator` 统一处理；
- `FrozenDict` 和 tuple 在校验边界还原为 dict 和 list，因此 immutable model
  保持 JSON 类型语义；
- Artifact 内容使用 strict JSON 解析，duplicate key、`NaN` 和 `Infinity`
  被判为执行错误；
- `format` 遵循 Draft 2020-12 默认 dialect 的 annotation 语义，assertion
  保持关闭；
- `pattern` 与 `patternProperties` 采用 `python-jsonschema` 的 Python
  regular-expression 语义；跨实现 Schema 使用 ECMA-262 与 Python `re`
  的公共语法；
- registry 的外部资源检索保持关闭；Schema 必须自包含，但 compound schema、
  `$defs`、anchor 和同文档引用可以使用。

推荐显式声明 dialect：

```python
from clef_sdk.verification import DRAFT_2020_12_URI

schema = {
    "$schema": DRAFT_2020_12_URI,
    "type": ["integer", "null"],
}
```

## VerificationContext

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.VerificationContext`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import VerificationContext
```

Namespace：`clef_sdk.verification`

输入字段：

- `task: SessionTask`；
- `workspace: Path`；
- `outputs: Mapping[str, ArtifactRef]`。

输出：verifier 接收的可信运行上下文。

## Verifier

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.Verifier`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import Verifier
```

Namespace：`clef_sdk.verification`

定义：

```python
Verifier = Callable[
    [VerifierSpec, VerificationContext],
    CheckResult,
]
```

输入：VerifierSpec 和 VerificationContext。

输出：CheckResult。

## VerifierRegistry

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.VerifierRegistry`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import VerifierRegistry
```

构造：

```python
VerifierRegistry()
```

方法：

```python
get(name: str) -> Verifier | None
names() -> tuple[str, ...]
register(
    name: str,
    verifier: Verifier,
    *,
    replace: bool = False,
) -> None
run(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult
```

输入：verifier 名称、verifier callable、VerifierSpec 和 VerificationContext。

输出：已注册 callable、名称 tuple、`None` 或 `CheckResult`。

## verify_outputs

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.verify_outputs`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import verify_outputs
```

```python
verify_outputs(
    task: SessionTask,
    outputs: Mapping[str, ArtifactRef],
    *,
    workspace: Path,
    registry: VerifierRegistry | None = None,
) -> VerificationReport
```

输入：SessionTask、已声明输出、workspace 和 registry。

输出：聚合 `VerificationReport`。

## visible_letter_number_count

**Canonical FQN（规范完全限定名）**：`clef_sdk.verification.visible_letter_number_count`

**Canonical import（规范导入）**：

```python
from clef_sdk.verification import visible_letter_number_count
```

```python
visible_letter_number_count(markdown: str) -> int
```

输入：Markdown 字符串。

输出：移除代码、链接语法和标记后的 Unicode 字母与数字数量。
