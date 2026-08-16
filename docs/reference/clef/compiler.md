# `clef_sdk.compiler` reference

本卷收录 4 个公共 API 和 4 个公共实体。所有符号均可从 `clef_sdk.compiler` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`compile_plan`](#compile_plan) | API | public API | 编译 WorkflowPlan 并生成 CompiledPlan |
| [`compile_workflow`](#compile_workflow) | API | public API | 编译 WorkflowPlan 并返回规范化计划 |
| [`CompiledPlan`](#compiledplan) | Entity | dataclass | WorkflowPlan 编译结果 |
| [`plan_digest`](#plan_digest) | API | public API | 计算 WorkflowPlan 与 Profile 的联合 digest |
| [`PlanIssue`](#planissue) | Entity | dataclass | 计划静态检查问题 |
| [`PlanValidationReport`](#planvalidationreport) | Entity | dataclass | 计划静态检查报告 |
| [`validate_plan`](#validate_plan) | API | public API | 对 WorkflowPlan 执行静态检查 |
| [`WorkflowCompileError`](#workflowcompileerror) | Entity | exception | WorkflowPlan 编译错误 |

## compile_plan

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.compile_plan`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import compile_plan
```

```python
compile_plan(
    definition: WorkflowPlan,
    profile: Profile,
) -> CompiledPlan
```

输入：

- `definition`：原始 WorkflowPlan；
- `profile`：已加载 Profile。

输出：`CompiledPlan`，包含规范化计划、plan digest、拓扑序、层级和验证报告。

异常：静态检查失败时抛出 `WorkflowCompileError`。

## compile_workflow

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.compile_workflow`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import compile_workflow
```

```python
compile_workflow(
    definition: WorkflowPlan,
    profile: Profile,
) -> WorkflowPlan
```

输入：WorkflowPlan 和 Profile。

输出：规范化并通过静态检查的 WorkflowPlan。

## CompiledPlan

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.CompiledPlan`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import CompiledPlan
```

Namespace：`clef_sdk.compiler`

输入字段：`plan`、`digest`、`profile_digest`、`topological_order`、`levels`、
`validation`。

输出：compile_plan 的完整结果。

## plan_digest

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.plan_digest`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import plan_digest
```

```python
plan_digest(
    plan: WorkflowPlan,
    profile: Profile,
) -> str
```

输入：WorkflowPlan 和 Profile。

输出：`sha256:<64 lowercase hex>`。

## PlanIssue

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.PlanIssue`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import PlanIssue
```

Namespace：`clef_sdk.compiler`

输入字段：

- `severity: Literal["error", "warning"]`；
- `code: str`；
- `message: str`；
- `task_id: str | None`。

输出：一个静态检查问题。

## PlanValidationReport

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.PlanValidationReport`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import PlanValidationReport
```

Namespace：`clef_sdk.compiler`

输入字段：`passed`、`issues`、`topological_order`、`levels`。

输出：validate_plan 的完整报告。

## validate_plan

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.validate_plan`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import validate_plan
```

```python
validate_plan(
    plan: WorkflowPlan,
    profile: Profile,
) -> PlanValidationReport
```

输入：WorkflowPlan 和 Profile。

输出：`PlanValidationReport`。

## WorkflowCompileError

**Canonical FQN（规范完全限定名）**：`clef_sdk.compiler.WorkflowCompileError`

**Canonical import（规范导入）**：

```python
from clef_sdk.compiler import WorkflowCompileError
```

Namespace：`clef_sdk.compiler`

输入：`PlanValidationReport`。

输出：compile_plan 的异常，`report` 属性保存完整静态检查报告。
