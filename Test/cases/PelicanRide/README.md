# Pelican Ride · Clef SDK 算例

这是 Simon Willison “Generate an SVG of a pelican riding a bicycle” 非正式
视觉基准的 agentic 扩展。原题只要求一张 SVG；本算例保留鹈鹕、双轮自行车和
骑乘空间关系，把它扩展成一个可玩的 Windows 桌面小游戏，并要求最终交付为
自包含的单文件 `PelicanRide.exe`。

> Historical case: this project targets the removed Clef Python `0.2` API and
> is retained as test material. It is not part of the current Haskell/Rust
> release gate; use the matching Git-history snapshot to execute it.

## 工作流

```text
┌────────────────────────┐
│ Vector art direction   │──┐
└────────────────────────┘  │
                            ├─> Playable prototype
┌────────────────────────┐  │          │
│ Gameplay architecture  │──┘          v
└────────────────────────┘      Independent review
                                        │
                                        v
                              Polish + single EXE
```

五个节点都由 Clef SDK 调度；每个节点使用独立 TaskRun workspace、
具名 Artifact、声明式 effect、确定性 verifier、失败后的 bounded repair 和
持久化 JSONL trace。视觉设计与玩法架构并行，其余节点只读取已验证的上游产物。

## 发起后台算例

在项目根目录运行：

```powershell
pwsh -NoProfile -File .\Test\cases\PelicanRide\start-pelican.ps1
```

脚本会返回唯一的 RunName、后台 PID 和监视命令。后台进程由 Windows WMI 创建，
不会依附当前终端；关掉发起窗口不会停止任务。

也可以指定便于记忆的名称：

```powershell
pwsh -NoProfile -File .\Test\cases\PelicanRide\start-pelican.ps1 `
  -RunName pelican-demo
```

同名输出绝不覆盖。需要重跑时使用新的 RunName，以保留完整证据链。

## 双层实时监视

另开一个 PowerShell：

```powershell
pwsh -NoProfile -File .\Test\cases\PelicanRide\watch-pelican.ps1 `
  -RunName pelican-demo
```

监视器每两秒刷新，分两层显示：

- `SDK / WORKFLOW`：运行状态、总进度、attempt、repair、verification 和发布数；
- `STUDIO / CELL LAYER`：每个 task/cell 的状态、当前 turn、attempt、repair 与耗时；
- `RECENT DURABLE EVENTS`：从 `workflow.jsonl` 读取的最近持久事件。

按 `Ctrl+C` 只会停止监视器，不会停止后台任务。原始输出同时保存在：

```text
Test/cases/PelicanRide/runs/live-<RunName>.stdout.log
Test/cases/PelicanRide/runs/live-<RunName>.summary.log
Test/cases/PelicanRide/.clef-state/output-<RunName>-*/traces/*/workflow.jsonl
```

## 最终产物

成功后：

```text
Test/cases/PelicanRide/runs/output-<RunName>/0400_delivery/delivery-bundle/
  PelicanRide.exe       # 双击即玩；无需安装 .NET
  preview.png           # 最终 EXE 自己渲染的代表帧
  README.md             # 操作说明
  verification.json     # 构建、smoke、preview、hash 与验收证据
  Source/               # 可复现源码，不是运行依赖
```

最终运行依赖只有 `PelicanRide.exe`。它是 .NET 8 WPF `win-x64` 自包含单文件；
微软运行库嵌入 EXE，不需要联网或同目录 sidecar。

诊断接口：

```powershell
.\PelicanRide.exe --smoke-test
.\PelicanRide.exe --render-preview C:\Temp\pelican-preview.png
.\PelicanRide.exe --version
```

## 断电与恢复

trace、已发布 Artifact 和任务 workspace 都是持久的。意外重启后可以定位最后一个
durable event 和完整的已完成节点；当前 Clef 协议尚未提供 workflow-level
原地续跑，因此未完成的 run 保留为取证记录，再用新 RunName 发起替代执行，避免
伪造连续性或覆盖旧证据。

## 来源

- Simon Willison, [Pelicans on a bicycle](https://simonwillison.net/2024/Oct/25/pelicans-on-a-bicycle/)
- [simonw/pelican-bicycle](https://github.com/simonw/pelican-bicycle)
- Robert Glaser, [Agentic Pelican on a Bicycle](https://www.robert-glaser.de/agentic-pelican-on-a-bicycle/)
