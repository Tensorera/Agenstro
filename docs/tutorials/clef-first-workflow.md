# 构建第一个 Clef SDK 工作流

本教程创建一个任务，通过 OpenCode 执行，并验证输出。它只覆盖第一次
Clef SDK 运行所需的路径。

## 1. 准备环境

- Python 3.12；
- Clef SDK 源码；
- OpenCode CLI；
- 一个可写 workspace；
- 一个可写 state 目录。

在项目根目录创建环境并安装包：

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\python.exe -m pip install -e .
opencode --version
```

项目元数据会安装 `jsonschema` 和 `referencing`。它们是内建
`json_schema` verifier 的 Draft 2020-12 实现；调用方无需单独补装依赖。
在继续之前，应已完成 OpenCode 认证并确认 `opencode` 可由当前 PowerShell
会话找到。

创建 Quickstart 目录：

```powershell
New-Item -ItemType Directory -Force quickstart
New-Item -ItemType Directory -Force quickstart\workspace
New-Item -ItemType Directory -Force quickstart\state
```

目录结构：

```text
quickstart/
  profile.toml
  run.py
  state/
  workspace/
```

## 2. 配置 Profile

创建 `quickstart/profile.toml`：

```toml
name = "quickstart"

[adapter]
executable = "opencode"
agent = "build"
output_format = "json"
pure = false
auto_approve = true
inherit_environment = true
required_env = []
extra_args = []

[runtime]
max_concurrency = 1
max_attempts = 1
retry_backoff_seconds = 0
fail_fast = true
max_subagent_depth = 1
max_fan_out = 8
session_reuse = "never"

[workspace]
root = "./workspace"
read_roots = []

[storage]
state_root = "./state"
cas_dir = "cas"
traces_dir = "traces"
cache_dir = "cache"
manifests_dir = "manifests"
cache_enabled = false
fsync = false

[labels]
environment = "quickstart"
```

Profile 路径字段以 `profile.toml` 所在目录为基准。加载结果为 immutable
`Profile`。

Clef Profile 不重复声明 shell、network、edit、delete 或 move 权限。
这些权限在 OpenCode 的 `opencode.json` 中通过 `permission` 配置。Clef 默认
设置 `adapter.auto_approve = true`，即为 `opencode run` 添加 `--auto`；
OpenCode 中显式配置的 `deny` 仍然有效。

如果当前阶段希望完全放行，在 OpenCode 的全局或项目级 `opencode.json` 中配置：

```json
{
  "$schema": "https://opencode.ai/config.json",
  "permission": "allow"
}
```

## 3. 编写任务

创建 `quickstart/run.py`：

```python
from pathlib import Path

from clef_sdk import (
    ArtifactKind,
    ArtifactSpec,
    DomainContract,
    EffectKind,
    EffectPolicy,
    EffectRule,
    Prompt,
    PromptRole,
    SessionTask,
    VerifierSpec,
    domain_run,
    load_profile,
)


ROOT = Path(__file__).resolve().parent
profile = load_profile(ROOT / "profile.toml")

task = SessionTask(
    id="hello",
    domain_function="quickstart.write_text.v1",
    prompts=(
        Prompt(
            role=PromptRole.INSTRUCTION,
            content=(
                "创建请求中名为 result 的文本 Artifact。"
                "正文写入 Clef SDK quickstart。"
                "完成后提交 SUCCEEDED AgentReport。"
            ),
            name="write-result",
            priority=10,
        ),
    ),
    outputs={
        "result": ArtifactSpec(
            name="result",
            description="Quickstart 文本结果",
            kind=ArtifactKind.TEXT,
            path="result.md",
        ),
    },
    contract=DomainContract(
        outputs={"result": ArtifactKind.TEXT},
        effects=EffectPolicy(
            allowed=(
                EffectRule(
                    kind=EffectKind.CREATE,
                    path_glob="result.md",
                ),
            ),
        ),
        verifiers=(
            VerifierSpec(
                name="min_text_length",
                parameters={"output": "result", "minimum": 10},
            ),
        ),
    ),
    metadata={"workspace_subdir": "hello"},
)

result = domain_run(task, profile=profile)

print(result.state.value)
print(result.outputs["result"].uri)
print(result.verification.passed)
```

## 4. 运行任务

```powershell
.\.venv\Scripts\python.exe quickstart\run.py
```

成功运行输出包含：

```text
SUCCEEDED
<absolute-path-to-result.md>
True
```

生成内容位于：

```text
quickstart/workspace/hello/result.md
```

运行状态位于：

```text
quickstart/state/
  cas/
  traces/
```

## 5. 核对结果流

```text
profile.toml
  -> load_profile
  -> Profile
  -> SessionTask
  -> domain_run
  -> AgentRequest
  -> OpenCodeAdapter
  -> AgentReport
  -> workspace audit
  -> verifier chain
  -> SessionResult
```

你已经创建 `SessionTask`，通过 `domain_run()` 执行，并验证一个声明的
Artifact。下一步：

- [Configure a Clef Profile](../how-to/configure-clef-profile.md)
- [Create a Clef verifier](../how-to/create-clef-verifier.md)
- [Clef API reference](../reference/clef-api.md)
- [Clef architecture](../explanation/clef/overview.md)
