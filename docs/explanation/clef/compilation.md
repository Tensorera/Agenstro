# 计划编译

本页解释 WorkflowPlan 的规范化、静态检查和稳定排序过程。精确函数签名与
CompiledPlan 字段以 [API 参考](../../reference/clef-api.md) 和
[实体参考](../../reference/clef-entities.md) 为准。

## 1. 入口

```python
compiled = compile_plan(plan, profile)
```

输入：WorkflowPlan 和 Profile。

输出：CompiledPlan。

## 2. 编译阶段

```text
WorkflowPlan
  -> normalize
  -> validate
  -> topological order
  -> level assignment
  -> profile binding
  -> plan digest
  -> CompiledPlan
```

## 3. Normalize

Normalize 处理：

- prompt 按 `priority` 降序排列，同一 priority 保持输入 tuple 的原始顺序；
- task workspace 由 Profile workspace 与 `metadata.workspace_subdir` 生成；
- ArtifactSpec.path 解析为 task workspace 内绝对路径；
- ResourcePolicy 保持 task 原始声明，不把 Profile runtime 上限写回 entity；
- mapping 和 sequence 转换为 immutable entity；
- path 文本执行规范化。

Normalize 输出新的 WorkflowPlan entity。

Profile 上限在使用点组合：scheduler 用 Profile 与 task 的较小
`max_attempts`，并用 Profile 与 workflow 的较小 `max_concurrency`。编译器只用
Profile 与 workflow 的较小 `max_fan_out` 执行静态 fan-out 检查。

## 4. Validate

Validate 检查：

- empty plan；
- binding source；
- binding target；
- output name；
- input name；
- ArtifactKind；
- JSON Schema Draft 2020-12 meta-schema；
- DomainContract 输入；
- DomainContract 输出；
- workflow 具名输出；
- workspace 路径；
- ContextRef；
- duplicate output path；
- cycle；
- named output 可达性；
- fan-out；
- 显式 logical effort 是否存在 Profile route；
- `opencode models` 成功返回时，所选模型是否存在。

每个问题生成 `PlanIssue`。

模型检查不会发送 prompt，也不会调用 `opencode run`。若 `opencode models`
不可执行、失败或输出无法解析，compiler 生成 `model_catalog_unavailable`
warning 并把最终判定延后到 fresh runtime turn；runtime 再次只查询 catalog。
仍无法确定或已知缺失时返回 typed、non-retryable 配置错误，不进入模型调用。

`max_subagent_depth` 当前由 Profile 和 WorkflowPolicies entity 校验为非负整数，
但编译器和 runtime 尚未计算或执行 subagent depth；不能把该字段当作已实施的
调度边界。

## 5. 拓扑序

拓扑排序先使用数值型 `task.metadata["order"]`，再使用 task ID 作为 ready
集合的稳定排序字段；非数值 order 按 `0` 处理。结果写入：

```python
CompiledPlan.topological_order
CompiledPlan.levels
```

`levels` 保存 `(task_id, level)` tuple。

## 6. Digest

plan digest 输入：

```text
protocol_version
normalized WorkflowPlan
Profile.digest
```

编码采用 UTF-8、key 排序、紧凑分隔符和标准 JSON number。输出格式：

```text
sha256:<64 lowercase hex>
```

## 7. 编译结果

```python
CompiledPlan(
    plan=normalized_plan,
    digest=plan_digest,
    profile_digest=profile.digest,
    topological_order=...,
    levels=...,
    validation=...,
)
```

WorkflowExecutor 使用 `CompiledPlan.plan` 进入调度。
