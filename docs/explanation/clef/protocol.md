# Agent 协议

本页解释 runtime 与 agent backend 之间的请求、报告和关联边界。精确常量、类型
与 codec 签名以 [API 参考](../../reference/clef-api.md) 和
[实体参考](../../reference/clef-entities.md) 为准。

## 1. 版本

```text
PROTOCOL_VERSION = "1.0"
SUPPORTED_PROTOCOL_VERSIONS = frozenset({"1.0"})
```

AgentRequest 和 AgentReport 都携带 `protocol_version`。

## 2. AgentRequest

字段：

```text
protocol_version
run_id
task_id
attempt
workspace
prompts
inputs
expected_outputs
allowed_effects
context
```

`allowed_effects` 保留既有字段名以维持协议结构，但语义是 task intent：
它帮助 agent 理解预期操作并帮助 trace 分类变化，不授予或拒绝工具权限。
OpenCode `permission` 是唯一授权来源。

编码：

```python
payload = encode_request(request)
request = decode_request(payload)
```

## 3. AgentReport

字段：

```text
protocol_version
run_id
task_id
attempt
text
state
artifacts
error
context
```

编码：

```python
payload = encode_report_json(report)
report = decode_report_json(payload)
```

## 4. Envelope

AgentReport envelope：

```text
<leading text>
REPORT_BEGIN_SENTINEL
<AgentReport JSON>
REPORT_END_SENTINEL
```

API：

```python
message = encode_report_envelope(report, leading_text=text)
report = decode_report_envelope(message, expected_request=request)
text, report = extract_report_envelope(
    message,
    expected_request=request,
)
```

decoder 选择消息末尾的完整 envelope。

## 5. JSON 规则

协议 JSON 使用：

- UTF-8；
- object key 排序；
- 紧凑分隔符；
- 标准 JSON number；
- 严格字段集合；
- protocol version 校验；
- entity 字段校验。

## 6. Correlation

expected_request 启用以下关联：

```text
report.protocol_version == request.protocol_version
report.run_id == request.run_id
report.task_id == request.task_id
report.attempt == request.attempt
```

关联结果进入 ProtocolCorrelationError。

## 7. ArtifactClaim

AgentReport.artifacts 中每项包含：

- name；
- URI；
- description；
- ArtifactKind；
- digest；
- media type。

runtime 使用 AgentRequest.expected_outputs 校验 ArtifactClaim。

## 8. ErrorInfo

失败 report 携带 ErrorInfo：

```text
code
category
message
retryable
details
cause
```

scheduler 使用 category、retryable、attempt 上限和 retry workspace 策略计算
retry；token/cost 观测不参与该决策。

## 9. ContextRef

ContextRef 字段：

```text
session_id
checkpoint_id
summary_artifact
message_range
```

计划上下文使用 summary_artifact 传递已经物化的信息。session 生命周期由 runtime
管理。

## 10. 协议错误

错误类型：

- ProtocolDecodeError；
- ProtocolValidationError；
- ProtocolCorrelationError；
- UnsupportedProtocolVersion。

DomainRunner 将协议错误作为可恢复失败：先在同 session 注入有界修复 prompt，
随后按 RecoveryPolicy 执行 compact 和 replacement session。
