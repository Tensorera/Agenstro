# Tactus architecture

This page explains Tactus's durability and recovery design. Use the
[CLI reference](../reference/tactus-cli.md), [runtime configuration](../reference/tactus-runtime-config.md),
and [compose contract](../reference/tactus-compose-contract.md) for exact
interfaces and limits.

## 定位与组件边界

Tactus Runtime 是一个 Windows-first、单工作流状态机，不是
`clef_sdk` 的可选模块，也不是完整的调度平台。它有独立的
`pyproject.toml`、依赖和运行目录，因此安装 Clef 不会安装 Jupyter、
pywin32 或 Agent CLI 运行组件。

```text
Agent Composer (Codex / OpenCode)
      │ ComposeDecision + complete workflow_memory
      ▼
Tactus engine ─── SQLite journal ─── main-thread.ipynb
      │                    │                    human projection
      │                    ├── phases / cells / events / interactions
      │                    ├── task state / interrupt request / warnings
      │                    └── recovery claim / workflow metadata
      │
      ├── JupyterCellExecutor
      │       └── WindowsJobProvisioner ─── fresh ipykernel process tree
      │
      ├── GitWorkspace ─── in-place project commits / rollback
      └── ResourceMonitor ─── path metadata/hash snapshots / user warnings

Tactus doctor ── read-only probes ── config / Git / SQLite / owners /
                                      kernelspec / provisioner / selected Agent
```

职责固定如下：

- Composer 的正常契约是只决定下一步，不在 compose 阶段写工作区。模型、
  provider、认证、sandbox 和 approval 策略由 Codex、Claude Code 或 OpenCode
  自己的配置负责；
  Tactus 不保存或覆盖这些偏好。严格只读需要用户在 Agent 中配置。
- Agent 权限只约束 compose 进程。fresh Jupyter kernel 是独立执行边界，
  Windows Job 只负责进程树生命周期，不提供文件或网络 sandbox。
- Engine 是唯一组合 SQLite、Git、资源快照和 Notebook 投影的组件。
- SQLite 是 cell/phase 生命周期和恢复证据的权威。
- Git 是 project mode 的主要 checkpoint/rollback backend；没有 Git 时仍可执行，
  但这项保证降级为不可用。
- `main-thread.ipynb` 是人可读投影，不是恢复权威。
- Jupyter/nbclient 执行一个 cell；Tactus 不自行实现 kernel 协议。
- helper script 承载确定性解析、验证和不需要 LLM 的逻辑。

## 当前项目模式与兼容模式

`motivo-studio` 的当前默认是 **in-place project mode**：

- 精确使用启动时的 cwd，不向上或向下搜索 `.tactus`；
- 没有 `.tactus` 时就在 cwd 中非破坏性初始化；
- Agent、cell、编译器和调试器都直接使用这个项目目录；
- Tactus 自有 script、helper、状态和 task JSON 全部位于 `.tactus`；
- 不创建临时镜像或 detached worktree；
- 非 Git 目录会询问是否在当前目录执行 `git init`，拒绝后仍可运行。

旧版 `tactus init --source --root` 创建的
`workspace_mode = "detached"` runtime 仍可读取。本文后续凡明确讨论 dedicated、
linked 或 detached worktree 的规则，只适用于该兼容模式；它们不是 Studio
当前启动行为。

## Artifact tree 与真实文件系统

Git 描述项目当前真实状态和 commit 历史；`.tactus/artifact-tree.json`
描述 Tactus 接受过的 task 如何让逻辑 artifact 集合演变。两者故意不被
假装成同一件事：外部 Agent、人工编辑和不可逆命令可能使真实文件系统与
Tactus 历史不同。

每个 task 对 tree 只产生三类净变化：`create`、`update`、`delete`；artifact
kind 只使用 `file` 或 `directory`。创建新 cell 时，显式引用的 artifact 会先
解析为项目相对路径并检查真实存在。Task 完成后，runtime 解析
`TaskResultEnvelope.artifacts`，将完整 JSON 存入
`.tactus/Artifact/tasks/`，再追加 tree revision。Delete 保留 tombstone，
不会从历史中抹除。

只有严格通过校验且显式标记为
`clef_sdk.task-result/v1` 的 envelope 可以修改 tree。未标记的
object、array、scalar 或 `null` 仍作为兼容结果归档，但 artifact 变化数为零；
未知保留版本、重复 JSON key、多余字段或不匹配的 kind 会使 cell 校验失败。
路径在所有平台都使用 Windows-safe、大小写不敏感的身份规则，当前存在的
artifact 不能同时以祖先目录和其后代路径重复登记。

`.tactus/Artifact/workspace/` 是方便人工查看的投影：file 使用硬链接，
directory 使用目录外壳加内部普通文件硬链接；不能安全链接时写诊断 pointer。
这个投影既不是第二份项目内容，也不是隔离/rollback backend。

## Phase、attempt 与 cell

一个 phase 表示主 Agent 认为已经完成后才应前进的任务阶段。一个 cell 是该
phase 的一次 attempt。

```text
phase 1
  attempt 1 / cell 1: FAILED       (永久保留)
  attempt 2 / cell 2: INTERRUPTED  (永久保留)
  attempt 3 / cell 3: SUCCEEDED    → phase 1 完成

phase 2
  attempt 1 / cell 4: DRAFT        → 仅这个 cell 可编辑
```

状态转换：

```text
                     compose edit
                    ┌────────────┐
                    ▼            │
new phase/retry ── DRAFT ────────┘
                    │ begin_run: freeze source
                    ▼
                 RUNNING
                  /  |  \
                 /   |   \
        checkpoint  rollback  host/containment loss
              ▼       ▼            ▼
         SUCCEEDED  FAILED     INTERRUPTED by recover
              │       │
       complete phase └── next compose appends same-phase attempt
```

关键约束由 SQLite unique index、foreign key 和 trigger 承担：

- 整个实例最多一个 active phase、一个 running cell；
- `(phase_id, attempt_no)` 唯一且不可变；
- phase 只能由属于它的 `SUCCEEDED` cell 完成；
- 已完成 phase 不可重新打开；
- schema v1 writer 不能绕过 v2 的 draft/run/terminal transition fence。

`begin_compose()` 会生成带 workflow revision、latest-cell token、objective 和
目标 phase/attempt 的 guard。Agent 返回后，`compose()`/`finish_workflow()`
在同一事务重新校验 guard，因此并发或过期 compose 结果不会覆盖新状态。新
phase ID 在 guard 中是“受保护的拟分配身份”，只有 `run` decision 提交时才
物化到 `phases` 表。

## 人机交互状态与任务中断

Tactus 把“人正在怎样与主 Agent 交互”和“自动 compose/run 是否可以继续”
建模为两个正交的持久状态轴，而不是把它们塞进 cell status：

```text
interaction_mode:  compose <──────────────> discussion
                              explicit start/end

task_state:        active ── interrupt completed ──> paused
                    ▲                                  │
                    └──────────── resume ──────────────┘

cell status:       draft → running → succeeded | failed | interrupted
```

Studio 在 cell status 之上显示更细的运行阶段，但这些阶段不是新的持久状态轴。
`status()` 在同一个 SQLite read transaction 中读取 cells、按 sequence 排序的
cell events 和 `snapshot_sequence`。Studio 根据 `run.kernel_launch_state`、
`run.execution_completed`、checkpoint/rollback/interruption events 与 terminal
cell status 推导 Draft composed、Kernel starting、Executing、Checkpointing、
Rolling back、Invalidating result 和 terminal 摘要；`run.containment_failed`
显示 Recovery required，未释放的 `recovery.claimed` 显示 Recovering。运行时约
每 650 ms 发起一个不重叠的 status poll；旧 sequence 快照被拒绝，未变化的 cell
摘要不会重复追加到 timeline。耗时、输出预览、错误和 interrupt 信息都能在
重开窗口后从 durable 记录重建。

`interaction_mode` 默认为 `compose`。用户可以在 compose mode 追加一条
`role=user, mode=compose` 的 durable message；它不调用 Agent，但会增加 workflow
revision，使已经在途且未看到该消息的 compose guard 无法提交。Discussion
只能在两个 cell execution 之间显式开始。进入 Discussion 后：

- `compose()`、`compose_with()` 和 `run_latest()` 都被状态机拒绝；
- 每个 human message 与 Agent reply 作为不可变 `interaction.message` event
  追加，reply 通过 `in_reply_to` 指向对应的 user message；
- `discuss_with()` 先把 ordinary workspace files 复制到一次性目录，排除
  `.git`、`.tactus` 和 `__pycache__`；遇到 symbolic link、junction、
  reparse point 或其他非普通文件系统条目时拒绝启动 Discussion；
- Discussion prompt contract 要求 Agent 只使用有界 context，不执行命令、不修改
  文件，也不声称已经执行代码；Codex Discussion 还强制
  `--sandbox read-only`，OpenCode 则在 disposable copy 内沿用其原生权限配置；
- mode 会跨多轮保持，直到用户显式结束 Discussion。

结束 Discussion 只把 `interaction_mode` 改回 `compose`，不会删除 transcript。
最近 50 条交互会进入下一次 compose context；因此讨论结论在回到 Compose 后仍
是 durable human steering。每条 context content 先限为 4,000 字符，并继续受
compose context 的 160,000 字符总预算约束。裁剪只影响发送给 Agent 的视图，
SQLite journal 仍保留原始消息。

一次性副本隔离的是 authoritative Tactus project；turn 结束后副本与其控制
目录会被删除。它不是网络或通用 host-resource sandbox，因此文档不把
Discussion 描述为网络隔离。

任务中断由独立的 `InterruptRequest` 表示，包含 `request_id`、mode、status、
目标 `cell_id` 和三个时间点。请求的正常生命周期是：

```text
requested → acknowledged → completed → Resume clears request
```

如果请求时没有 running cell，请求直接成为 `completed`，并把 `task_state`
设为 `paused`。如果已有 running cell：

- `graceful` 不废弃当前 attempt。cell 继续走正常 execution、checkpoint 或
  rollback 路径；到达任一 terminal status 后请求才完成，任务随后暂停。
- `immediate` 可以新建请求，也可以升级尚未完成的 graceful 请求。executor
  在执行与提交边界反复检查该请求；观察到后终止 execution，把 cell 标为
  `interrupted`，清除 durable outputs 并回滚 tracked files。非 Git 资源的
  创建者无法被可靠证明，因此一律保留；与 cell 前快照的差异产生资源
  warning，交由用户检查。

Immediate 表示“当前 cell 的结果无效”，不是绕过 containment 的强制 reset。
Engine 仍必须先证明 kernel 清理和 rollback 安全；无法证明时 cell 保持
`running`，交给 recovery，而不会并发破坏工作区。

两种模式最终都会令 task paused。`resume` 只在没有 running cell、且请求不再
处于 `requested`/`acknowledged` 时恢复 `active` 并清除 completed request。
Resume 不会重跑 immediate stop 的 cell；下一次 compose 按普通失败/中断规则
创建同一 phase 的新 attempt。`loop` 在 compose/run 边界读取这一状态，并在
paused 时以 interrupted 结果停止。

## 无状态兼容命令的持久上下文

旧的 `tactus compose/loop` 每次都是无状态请求；Codex 使用 ephemeral
exec，Claude Code 与 OpenCode 使用独立的非交互请求。这些兼容命令的输入由
Tactus 重新组装：

- 不可裁剪的 objective、current attempt identity 和 allowed actions；
- 限长 cell history，最近 8 个 cell 带详细 source/output/error；
- 有界 tracked/helper/imported-resource 路径；
- 完整 `workflow_memory`；
- 最近的 durable human/agent interaction history。

当前生产 Studio 不使用这条 Agent Gateway 路径。用户在 xterm.js 渲染的
PowerShell 或 Bash PTY 中手动启动原生 coding agent，项目级
`AGENTS.md`/`CLAUDE.md` 提供 prompt-only 指导，
`.tactus/main_script.py` 是 Studio 与 agent 共享的直接脚本。Studio 不维护
provider session、context epoch 或 ACK。

`workflow_memory` 是严格 schema，而不是自由增长的聊天摘要：

```text
current_state   <= 6000 chars
decisions       <= 20 items
open_issues     <= 20 items
artifacts       <= 30 typed items
validation      <= 20 items
```

Agent 必须返回完整的新 memory。它与 cell compose 或 workflow completion
原子提交，context 总序列化大小另有硬上限。资源回滚 warning 刻意不进入
Agent context，只面向用户显示。

这降低 compact 丢失关键状态的概率，但不替代业务数据库：大结果、结构化数据
和可验证 artifact 仍应写入工作区文件或显式数据库。

## 运行提交协议

### 开始

1. 从 Notebook 只同步最新 draft 的 source，并重建 canonical 投影。
2. Project mode 提交现有 non-ignored 变更为 pre-cell baseline；legacy mode
   验证 dedicated worktree 干净。
3. 记录 `base_commit`、helper digest、非 Git 资源快照和 execution notebook
   路径。
4. SQLite 将 cell 从 draft 原子切换为 running，并记录 PID/host。
5. 启动 fresh kernel。

### 执行成功

1. 原子写完整 execution notebook。
2. SQLite 写 `run.execution_completed(succeeded=true)`。
3. 资源快照差异写入 cell，并记录 `run.checkpoint_prepared`。
4. Project mode 提交完整 non-ignored 变更；legacy mode 清除 cell 私自做出的
   staging 决策并按其 text/include policy 创建 checkpoint。两者都产生精确
   single-parent、精确 message 的 allow-empty commit。
5. 再次验证 execution notebook、index/worktree clean 和 HEAD。
6. SQLite 在一个事务中 terminalize cell、完成 phase、写事件和 warning。
7. Notebook 投影 best-effort 重建；投影失败不能撤销已提交的 terminal state。

compose 提交和 `begin_run` 之后的 Notebook 更新也遵守同一规则：SQLite
转换成功后，投影异常只写 `PROJECTION_UPDATE_FAILED` 用户告警，不能让 API
把已提交的 cell 伪装成失败，更不能在 kernel 尚未启动时把 cell 卡死。

### 执行失败

1. execution notebook 和失败结果先持久化。
2. 记录 execution-time resource diff 与 `run.rollback_prepared`。
3. Git 回到 `base_commit`；project mode 不运行全项目 `git clean`，也不
   猜测新增 untracked 路径属于 cell、用户还是并行 agent。所有 untracked
   内容保留；若它新增、修改、重命名或删除，则产生非阻塞人工检查告警。
4. SQLite 原子把 cell 标为 failed，phase 保持 active，warning 同事务提交。
5. 下一次 compose 只能创建同一 phase 的 `attempt_no + 1`。

若 checkpoint 自身失败，Tactus 先回滚，再用独立证据
`run.checkpoint_failed` terminalize 为 failed。若 checkpoint 和 rollback
都失败，cell 保持 running 等待 recovery。

## Kernel containment

Tactus 通过 Jupyter Client 的 `kernel_provisioners` entry-point 使用
`WindowsJobProvisioner`。它继承 Jupyter 的 `LocalProvisioner`，没有重写
消息协议或 notebook 执行逻辑。

0.1 只接受标准 direct Python kernelspec：

```text
python -m ipykernel_launcher -f {connection_file}
```

provisioner 不直接 Popen ipykernel，而是先启动
`python -I -S kernel_gate_runner.py`。这个 stdlib-only runner 只阻塞读取
继承的 stdin gate 中的随机精确 token，不导入 Tactus、site packages 或
用户代码；host 退出会关闭 pipe，使未放行 runner 收到 EOF 并退出。runner
进入设置了 kill-on-close 的 Windows Job Object、`contained` 已经 durable
提交之后，host 才通过 pipe 写入精确 `GO` token 并关闭；runner 随后以原始 argv、env、
cwd、stdio、parent handle 和 interrupt event 启动 direct ipykernel 并等待。
因此任意 `sitecustomize` 只可能在 Job 内执行，不再存在 ipykernel 的
post-Popen assignment 逃逸窗口。

如果 cleanup 仍不能证明 Job handle 已关闭且 kernel 已回收，executor 抛出
`KernelContainmentError`。Engine 只写
`KERNEL_CONTAINMENT_FAILED`，保持 cell running，绝不在可能仍有 writer 的
情况下并发 rollback。宿主终止后由 recover 接管。

每次首次启动或 Jupyter 显式 restart 都使用新的 `launch_id`。provisioner 在
runner Popen 前同步提交带 `protocol=gated-runner-v1` 的 `starting`，Assign
Job 成功后再为同一 generation 提交 `contained`，之后才释放 gate；cleanup
失去收容保证时提交
`containment_failed` 及其恢复策略。recovery 只信任最新 state，因此旧
generation 的成功证据不能覆盖新一轮。owner 退出后，最新 gated `starting`
也可安全 rollback：durable `contained` 不存在即证明 gate 尚未由可信 host
释放，runner 不可能启动用户 kernel。

失败清理只有在 Job accounting 报告 `ActiveProcesses=0`，或 gate 尚未释放且
runner 已停止时才算 proven。查询失败、released gate 下无法证明 Job 为空等
情况都会保留 sticky `KernelContainmentError`。

runner 在读 GO 前取得 root control 下固定的 share-deny
`kernel-operation.lock`，actual ipykernel 继承该 handle。Job 同时使用由
`launch_id` 唯一派生的 named Job object。recovery 在任何 resource/Git 读取前
必须同时取得并释放固定 lease，并确认旧 named Job 不存在或
`ActiveProcesses=0`；`--force` 不绕过这两个 live-process fence。named Job
probe 覆盖 Job 内未继承 lease 的任意后代。

## Crash recovery 与并发恢复

recover 不是“看到 running 就 reset”。顺序如下：

1. 在 `BEGIN IMMEDIATE` 中检查原 run owner。
2. 创建含随机 token、PID、host、process start bound 的 recovery claim。
3. 第二个 recover、旧 token 和无 token 的旧 runner 都被 Store 写屏障拒绝。
4. 先检查最新 kernel launch-generation 证据；证据不足则在读取 workspace
   资源快照或 Git 状态前停止。
5. 读取 durable execution/checkpoint/rollback evidence、资源快照和 Git HEAD。
6. 只有精确匹配 base + checkpoint label + one-parent 关系的 commit 才能被
   reconcile 为 succeeded。
7. destructive rollback 前后都重新校验 claim。
8. finish 在同一事务 terminalize cell/phase、写 warning 并清 claim，成功后
   不再执行第二次 claim 写事务。
9. 未 terminalize 的异常路径按 token CAS 释放；硬崩溃由下一进程确认 owner
   stale 后接管。

claim 不使用纯时间 TTL。慢 IO 或暂停的 recover 仍可能恢复执行并操作 Git，
所以不能仅因 lease 到期就让另一个进程抢占。可证明 alive 的 run 或 recovery
owner 即使在 `--force` 下也不会被偷取。

workflow 写入 `complete` 后，普通 compose 不允许隐式重新打开。compose
context 会携带 completion/status，允许动作只剩 `finish`；重复 `loop` 在调用
Agent 前直接返回已有完成记录。周期性任务必须由上层 occurrence host 分配
新的 root，而不是复用上期 cell 历史。

## SQLite schema v2 与迁移

v2 新增：

- `phases`；
- cell 的 `phase_id`、`attempt_no`；
- bounded `workflow_memory`；
- Notebook projection version；
- transition fences 和 recovery claim metadata。

v1→v2 只能由显式命令启动：

```powershell
tactus migrate --root $orch --confirm-hosts-stopped
```

普通 load、status、run、loop 和 recover 只验证 schema，绝不隐式迁移。
confirmation 表示用户已经停止所有持有该 root 的 v1 host；它不是 kill 或
抢占操作，可证明 alive 的 running owner 仍会令迁移失败。

Legacy migration 命令会先确认 config 对应的工作区确实是专用 linked
worktree；该检查失败时还
不会打开迁移事务。确认通过后，迁移在一个 `BEGIN IMMEDIATE` 事务内验证 cell
ordinal、previous/retry 链、succeeded timestamp、workflow status/objective、
completion history、memory 和 revision；随后确定性回填 phase/attempt，并
归一化 legacy completion。最后执行 foreign-key/quick check，只有全部成功才把
schema version 写为 2。任何错误都回滚为纯 v1。

迁移后的 trigger 会阻止迁移前已经加载的 v1 writer 再执行
draft/run/terminal 写入。由于 v1 没有 launch-generation 证据，迁移后的
running cell 不会自动进入 Git/resource recovery；用户核实旧进程结束后仍需
显式 `recover --force`。

## 初始化边界

Project bootstrap 精确检查 `<cwd>/.tactus/config.json`。若配置缺失，则创建
项目配置、helper、Artifact 目录和内部 ignore 规则；若配置存在，则补齐缺失
布局并恢复。它不会因为项目目录非空而拒绝，也不会寻找相邻项目。SQLite、
Artifact tree 和首次 Notebook 投影之间若发生宿主硬退出，下一次同目录启动可
继续补齐布局，但 production occurrence host 仍应记录自己的初始化幂等键。

Legacy detached bootstrap 仍涉及 worktree add/lock、资源复制和外部 runtime
root；其部分初始化诊断只适用于兼容模式。

## Notebook 投影

`main-thread.ipynb` 始终从 SQLite 全量重建：

- 所有历史 code cell 都 `deletable=false`；
- 只有最新 draft `editable=true`；
- cell metadata 投影 phase/attempt/status/commit/execution notebook；
- raw JSON 修改历史 source、ID、顺序或插入 cell 会被拒绝；
- 丢失、过旧或部分写入的投影可以由 `rebuild-notebook` 修复。

Notebook 更新发生在权威事务之后，因此 projection failure 只产生 warning。
`status`、`doctor` 和 `recover` 不依赖一个可解析的 Notebook 才能读取权威
状态。

## Git 与非 Git 资源边界

Project mode：

- 现有 Git root 的设置保持不变；
- cell 前将当时全部 non-ignored 变更提交成安全 baseline；
- 成功 cell 提交全部 non-ignored 项目变更，即使没有内容变化；
- 失败 cell `reset --hard` 到 baseline，并只清理该 cell 新增的 non-ignored
  路径；
- ignored 文件及外部副作用不受 Git 回滚；
- 没有 Git 时使用 `git-disabled` 边界继续运行，但不声称已经回滚。

Legacy detached mode：

- worktree 必须 detached、linked 且被 lock，不允许 primary worktree；
- tracked rollback 使用 mixed reset + restore；
- 新文件按 text/include 策略选择性接纳。

运行期 Git 还有独立的进程一致性协议：

1. 取得按 worktree 标识的 Win32 share-deny lease，并协调所有 Git 读写；
2. 写 `starting` operation marker；
3. 启动只等待精确 `GO` 的 isolated Python runner；
4. runner 进入 kill-on-close Job 后写 `contained`，随后才发送 `GO`；
5. runner 与 Git 都继承 lease，Git hooks、auto maintenance 和 auto-gc 被关闭，
   会重定向 repository/index 的 `GIT_*` 环境变量被清除；
6. 正常/异常清理必须证明 Job `ActiveProcesses=0`，再删除自身 marker、释放
   lease；无法证明时 handle 和 lease 在宿主进程内保持 sticky；
7. 新进程取得旧 lease 后，才可 reconcile 已知 marker。旧 mutating operation
   留下的 worktree-specific lock 会移动到该 linked worktree admin dir 下的
   quarantine；unknown marker 与 shared common-dir lock 一律保留并拒绝继续。

Project mode 不调用 `git worktree add`。Legacy detached mode 的首次
repository discovery 和 `git worktree add` 仍可能触碰 shared common-dir
metadata，属于其兼容 bootstrap 边界。

因此大 PDF、模型、数据库和二进制输入若需要强恢复，必须进入显式 artifact
store、用户指定 Git 策略，或未来的隔离 backend。Tactus 只向用户报告
无法 checkpoint/rollback 的变化，不能声称已经恢复。

## Doctor 的只读边界

Doctor 不调用 `Tactus.load()`，所以不会迁移、recover 或推进状态。它使用
SQLite `mode=ro + query_only`，检查 v1/v2 storage、run/recovery owner、
当前 workspace mode/Git 边界、依赖版本、direct ipykernel、provisioner entry point 和
所选 Agent launcher，并校验 recovery claim 与唯一 running cell、active phase 与
最新未完成 cell、succeeded cell 与 phase completion 的引用关系，但不启动
kernel/Agent。WAL reader 仍可能使用
`-wal/-shm` sidecar，所以运行目录只支持本地磁盘。

## 尚未解决，必须放在独立组件

1. 定时 trigger、workflow occurrence、幂等键、输入冻结和通知 host。
2. 多 subagent 的 task/worker/result schema、claim、汇合与失败传播。
3. 内容寻址 artifact store、资源配额、保留策略与垃圾回收。
4. 可选隔离目录 backend、远程 worker 和多机 lease。
5. JupyterLab、远程浏览器客户端以及跨设备 Agent session handoff。

其中 1/2 是 Tactus 之上的 host/worker plane，不应回填进
`tactus-runtime`。Runtime 只提供可组合的单工作流 compose/run/recover
状态机。
