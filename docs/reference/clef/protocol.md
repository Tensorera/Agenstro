# `clef_sdk.protocol` reference

本卷收录 7 个公共 API 和 11 个公共实体。所有符号均可从 `clef_sdk.protocol` 直接导入。

返回：[API index](../clef-api.md) · [Entity index](../clef-entities.md)

**符号索引**

| Symbol | Catalog | Type | 一行说明 |
| --- | --- | --- | --- |
| [`AgentReport`](#agentreport) | Entity | dataclass | agent 返回协议对象 |
| [`AgentRequest`](#agentrequest) | Entity | dataclass | runtime 发出的协议对象 |
| [`decode_report_envelope`](#decode_report_envelope) | API | public API | 从 sentinel envelope 解码 AgentReport |
| [`decode_report_json`](#decode_report_json) | API | public API | 从严格 JSON 解码 AgentReport |
| [`decode_request`](#decode_request) | API | public API | 从严格 JSON 解码 AgentRequest |
| [`encode_report_envelope`](#encode_report_envelope) | API | public API | 将 AgentReport 编码为 sentinel envelope |
| [`encode_report_json`](#encode_report_json) | API | public API | 将 AgentReport 编码为规范 JSON |
| [`encode_request`](#encode_request) | API | public API | 将 AgentRequest 编码为规范 JSON |
| [`extract_report_envelope`](#extract_report_envelope) | API | public API | 提取消息正文和 AgentReport |
| [`PROTOCOL_VERSION`](#protocol_version) | Entity | constant | 当前协议版本 |
| [`ProtocolCorrelationError`](#protocolcorrelationerror) | Entity | exception | 协议关联错误 |
| [`ProtocolDecodeError`](#protocoldecodeerror) | Entity | exception | 协议解码错误 |
| [`ProtocolError`](#protocolerror) | Entity | exception | 协议错误基类 |
| [`ProtocolValidationError`](#protocolvalidationerror) | Entity | exception | 协议字段校验错误 |
| [`REPORT_BEGIN_SENTINEL`](#report_begin_sentinel) | Entity | constant | AgentReport 起始标记 |
| [`REPORT_END_SENTINEL`](#report_end_sentinel) | Entity | constant | AgentReport 结束标记 |
| [`SUPPORTED_PROTOCOL_VERSIONS`](#supported_protocol_versions) | Entity | constant | 支持的协议版本集合 |
| [`UnsupportedProtocolVersion`](#unsupportedprotocolversion) | Entity | exception | 协议版本错误 |

## AgentReport

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.AgentReport`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import AgentReport
```

Namespace：`clef_sdk.protocol`

输入字段：

- `run_id: str`；
- `task_id: str`；
- `attempt: int`；
- `text: str`；
- `state: RunState`；
- `artifacts: tuple[ArtifactClaim, ...]`；
- `error: ErrorInfo | None`；
- `context: ContextRef | None`；
- `protocol_version: str`。

输出：agent 完成消息的协议 entity。

## AgentRequest

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.AgentRequest`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import AgentRequest
```

Namespace：`clef_sdk.protocol`

输入字段：

- `run_id: str`；
- `task_id: str`；
- `attempt: int`；
- `workspace: str`；
- `prompts: tuple[Prompt, ...]`；
- `inputs: tuple[ArtifactRef, ...]`；
- `expected_outputs: tuple[ArtifactSpec, ...]`；
- `allowed_effects: EffectPolicy`；
- `context: ContextRef | None`；
- `protocol_version: str`；
- `effort: Effort | None`。

输出：runtime 发送给 adapter 的协议 entity。为 `None` 时规范 JSON 不写入
`effort`，保持 protocol v1 的旧 payload；显式值表示 Clef logical route，
不是 provider-native effort。

## decode_report_envelope

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.decode_report_envelope`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import decode_report_envelope
```

```python
decode_report_envelope(
    message: str,
    *,
    expected_request: AgentRequest | None = None,
) -> AgentReport
```

输入：包含 report sentinel 的消息和可选关联请求。

输出：通过协议校验和关联校验的 `AgentReport`。

## decode_report_json

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.decode_report_json`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import decode_report_json
```

```python
decode_report_json(
    payload: str,
    *,
    expected_request: AgentRequest | None = None,
) -> AgentReport
```

输入：AgentReport JSON 和可选 AgentRequest。

输出：`AgentReport`。

## decode_request

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.decode_request`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import decode_request
```

```python
decode_request(payload: str) -> AgentRequest
```

输入：AgentRequest JSON。

输出：`AgentRequest`。

## encode_report_envelope

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.encode_report_envelope`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import encode_report_envelope
```

```python
encode_report_envelope(
    report: AgentReport,
    *,
    leading_text: str | None = None,
) -> str
```

输入：AgentReport 和可选正文。

输出：带 begin/end sentinel 的消息字符串。

## encode_report_json

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.encode_report_json`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import encode_report_json
```

```python
encode_report_json(report: AgentReport) -> str
```

输入：AgentReport。

输出：规范 JSON 字符串。

## encode_request

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.encode_request`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import encode_request
```

```python
encode_request(request: AgentRequest) -> str
```

输入：AgentRequest。

输出：规范 JSON 字符串。

## extract_report_envelope

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.extract_report_envelope`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import extract_report_envelope
```

```python
extract_report_envelope(
    message: str,
    *,
    expected_request: AgentRequest | None = None,
) -> tuple[str, AgentReport]
```

输入：agent 消息和可选 AgentRequest。

输出：消息正文与 AgentReport。

## PROTOCOL_VERSION

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.PROTOCOL_VERSION`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import PROTOCOL_VERSION
```

Namespace：`clef_sdk.protocol`

值：`"1.0"`。

输出：encode 函数和协议 entity 使用的当前协议版本。

## ProtocolCorrelationError

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.ProtocolCorrelationError`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import ProtocolCorrelationError
```

Namespace：`clef_sdk.protocol`

输入：异常消息。

输出：report 的 run、task 或 attempt 与请求关联校验产生的
`ProtocolValidationError`。

## ProtocolDecodeError

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.ProtocolDecodeError`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import ProtocolDecodeError
```

Namespace：`clef_sdk.protocol`

输入：异常消息。

输出：协议 JSON 或 sentinel 解码产生的 `ProtocolError`。

## ProtocolError

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.ProtocolError`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import ProtocolError
```

Namespace：`clef_sdk.protocol`

输入：异常消息。

输出：协议错误基类实例。

## ProtocolValidationError

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.ProtocolValidationError`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import ProtocolValidationError
```

Namespace：`clef_sdk.protocol`

输入：异常消息。

输出：协议 entity 字段校验产生的 `ProtocolError`。

## REPORT_BEGIN_SENTINEL

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.REPORT_BEGIN_SENTINEL`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import REPORT_BEGIN_SENTINEL
```

Namespace：`clef_sdk.protocol`

值：

```text
<<<CLEFFRAMEWORK_AGENT_REPORT_BEGIN_V1_7D25EAE9E11B4D81A404D6E36FA12C71>>>
```

输出：AgentReport envelope 起始标记。

## REPORT_END_SENTINEL

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.REPORT_END_SENTINEL`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import REPORT_END_SENTINEL
```

Namespace：`clef_sdk.protocol`

值：

```text
<<<CLEFFRAMEWORK_AGENT_REPORT_END_V1_7D25EAE9E11B4D81A404D6E36FA12C71>>>
```

输出：AgentReport envelope 结束标记。

## SUPPORTED_PROTOCOL_VERSIONS

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.SUPPORTED_PROTOCOL_VERSIONS`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import SUPPORTED_PROTOCOL_VERSIONS
```

Namespace：`clef_sdk.protocol`

值：`frozenset({"1.0"})`。

输出：decode 函数接受的协议版本集合。

## UnsupportedProtocolVersion

**Canonical FQN（规范完全限定名）**：`clef_sdk.protocol.UnsupportedProtocolVersion`

**Canonical import（规范导入）**：

```python
from clef_sdk.protocol import UnsupportedProtocolVersion
```

Namespace：`clef_sdk.protocol`

输入：异常消息。

输出：协议版本集合校验产生的 `ProtocolValidationError`。
