# Test2：缺失 Supplementary 的盲复现基准

本目录把 `Testarticle.pdf` 变成一个可重复执行的 Clef SDK 案例：
在不下载论文真实 supplementary、不联网补资料的前提下，从正文推导可识别的
补充内容，用独立数值程序复核，并把不可识别部分保留为 `BLOCKED`。

它测试的是框架能力，而不只是一段论文计算脚本：

- 真实编译并执行一个 `1 -> 3 -> 1` 的 `WorkflowPlan`；
- 三个中间任务在证据盘点后可并行调度；
- 每个 TaskRun 产出经过 JSON Schema、文本长度和领域 verifier 的 Artifact；
- 数值 Artifact 会由 verifier 独立重算，不能只靠 agent 自报正确；
- 数值报告被篡改时，下游综合任务必须跳过；
- 离线回归与真实 agent 模式使用同一计划和同一 verifier registry。

## 三层执行证据与四层状态

三种证据不能互相代替：

1. `output-final` 是 `FakeAdapter` 生成的 deterministic offline reference，
   证明固定输入能够走通 DAG、公开合同、verifier、发布、fan-in 和 digest
   流程；它不证明 live agent 能自主推导这些内容。
2. `--mode agent` 用 OpenCode 在 blind bundle 上执行同一计划，观察 live agent
   的生成、修复和收敛行为。一次 live run 必须按自己的 run ID 和 trace 判断，
   不能继承 offline reference 的成功。
3. host-side verifier 在 agent 之外独立检查合同、证据边界、数值与 digest。
   verifier 通过证明候选满足本 benchmark 的本地判据，不等于出版商 SI 身份
   比对。

因此一次结果至少有四个正交层次：

| 层次 | `output-final` 当前含义 |
| --- | --- |
| Workflow execution | 5 个 TaskRun 均完成，`workflow_state=SUCCEEDED` |
| Clef verification | 当前本地 Schema、公开合同、领域检查和 digest 通过 |
| Scientific assessment | `partial_reproduction`，有 3 个不可识别项保持 `BLOCKED` |
| Historical SI identity | `historical_identity_verified=false`；未取得真实 SI，历史表达一致性 **not evaluated** |

Manifest 中的 `clef_verified` 只表示对应固定槽 Artifact 通过上述本地合同与
digest 检查。它不表示内容由 live agent 生成，不表示完整科学复现，也不表示与
作者历史 SI 逐式相同。

早期真实 agent run 暴露了一个 benchmark 问题：理论 verifier 曾要求 agent
看不到的固定结构 ID。现在这些非答案性的结构要求位于 blind bundle 内的公开
`theory_output_contract`，verifier 也读取同一份不可变合同；数值 golden 和
反伪造校验仍只留在 host 侧。这项设计修正不构成后续 live run 成功的证据。详见
[live-run-lessons.md](live-run-lessons.md)。

## 五类公开 output contract

`benchmark-spec.json` 把 producer 可见的要求分成五类，分别对应五个 DAG
节点：

| Contract | 公开内容 |
| --- | --- |
| `evidence_output_contract` | 固定 source identity、禁止联网/SI、四类 SI dependency 的 ID、锚点与 recoverability |
| `theory_output_contract` | polynomial 与 oblique 两节的结构 ID、状态、证据/报告 token 和禁止夸大的结论 |
| `methods_output_contract` | 正文可确认的实验/FEA facts、Fig. S1 边界、缺失字段类别和禁止结论 |
| `numeric_output_contract` | check ID、状态、observed/expected JSON 路径、比较方式、公开容差与证据锚点 |
| `final_output_contract` | 最终报告章节、blind 声明、3 个 blocker、固定 manifest slots 与禁止结论 |

`numeric_output_contract` 只公开结果结构、比较方式和容差，不公开 expected
golden values。goldens、独立重算实现和反伪造检查仍在 host 侧；这样 agent
知道怎样提交可验证结果，却看不到答案。

## 科学边界

固定输入只有：

- `Test/cases/Test2/Testarticle.pdf`；
- 与该 PDF 摘要绑定的 `Test/cases/Test2/review-work/Extractedmd/full.md`；
- 既有本地审稿报告；
- `benchmark-spec.json`。

正文中的 Appendix A 位于论文 PDF 内，不是缺失的在线 SI。正文真正委托给 SI
的内容至少包括：

1. 斜压缩方程 (47)-(48) 的 `T_I...T_IV` 解函数；
2. 多项式初始形状的一阶扭转结果；
3. 实验和 ABAQUS 方法细节；
4. 含基底与不含基底模型的 Fig. S1。

deterministic offline reference 的科学判定为 `partial_reproduction`：

- 9 项 `PASS`、0 项 `FAIL`：`SRC-001/002` 是 2 项输入身份，
  `SI-SCOPE-001` 是 1 项 SI 范围盘点，另外 6 项才是数学/数值科学检查
  (`NUM-BMAX-001`、`NUM-FIG5-001`、`NUM-FIG8-001`、
  `NUM-PRESTRAIN-001`、`NUM-FIG10-001`、`NUM-STRAIGHT-001`)；
- 多项式 SI 缺失的 `phi_(1)b(1)`、`U_2(2)b(1)`、
  `kappa_3_(1)b(1)` 是从正文一般方程得到、与正文一致并经检查的
  **candidate reconstruction**；它们通过独立积分样点、微分恒等式、端点条件
  和式 (71) 模态比交叉验证，但未与真实 SI 对照，不能宣称与作者历史 SI 的
  符号形式、积分常数选择或表达完全一致；
- Fig. 5 截断误差、三类 `b_max`、Fig. 8 模态比、预拉伸换算、
  Fig. 10 反设计近似和直梁极限均可独立重算；
- 作者历史上采用的斜压缩函数基归一化、Fig. S1 的 ABAQUS 全场结果、
  实验原始坐标与统计量共 3 项保持 `BLOCKED`。

`BLOCKED` 不是失败，也不能被 agent 用“常见”的网格、材料卡、样本量或特殊函数
归一化填空。获得作者 SI、ABAQUS 输入和实验原始数据后，才可升级这些结论。

正文把 40% applied compression 描述为约等价于 `~66.7%` substrate
prestrain。这里只验证定义换算 `p/(1+p)`：若 `66.7%` 是精确 `2/3` 的
四舍五入，则结果约为 40%；将字面十进制 `p=0.667` 代入并不严格等于 0.4。
该 PASS 是代数换算检查，不是 Fig. S1、材料响应或实验结果的独立复现。

## DAG

```text
inventory-supplement-evidence
              |
      +-------+--------+
      |       |        |
 infer-    infer-   validate-
 theory    methods  numerics
      |       |        |
      +-------+--------+
              |
synthesize-inferred-supplement
```

实际计划有 5 个 TaskRun、11 条 Artifact binding。冻结的输入策略、必验数值项、
预期阻塞项和框架探针见 `benchmark-spec.json`。

## 运行

命令均从仓库根目录执行。只编译计划：

```powershell
python Test\cases\Test2\reproduction\run.py --plan-only `
  --workfolder Test\cases\Test2\reproduction\plan-output
```

运行确定性的离线框架回归：

```powershell
python Test\cases\Test2\reproduction\run.py --mode offline `
  --workfolder Test\cases\Test2\reproduction\output-final
```

离线模式不是空文件占位：`FakeAdapter` 会生成完整的证据、推导和数值
Artifact，再由 Clef 完成隔离执行、验证、发布、fan-in 与失败传播。它适合
CI，因为不读取 `.env`、不联网、也不启动收费 agent。这里的“完整 Artifact”仍是
deterministic fixture，不是自主 agent 生成证据；`output-final` 应被称为 offline
reference。

执行时，runner 会把 SDK 层的有界实时摘要写到 `stderr`：从 plan 编译开始，
逐项显示 task/attempt、agent session、verification、repair、publication 与
当前完成数；prompt、候选正文和完整 verifier report 不会进入摘要。最终
`WorkflowRunSummary` 同时写入 `run-summary.json` 的 `execution_summary`，
原始 workflow/attempt JSONL 仍是审计证据。

使用 OpenCode agent 执行同一 DAG：

```powershell
python Test\cases\Test2\reproduction\run.py --mode agent `
  --profile Test\cases\Test2\reproduction\reproduction_profile.toml `
  --workfolder Test\cases\Test2\reproduction\agent-output
```

真实 agent 模式要求 profile 中的 `opencode`、模型和 OpenCode 本地凭据存储已经
可用；出于盲测隔离，runner 不接受通过 agent 子进程环境透传的 API key。runner 会从
每次 run 创建不可变的 `0000_blind_inputs` allowlist bundle；其内容只有论文、
固定 OCR、明确允许的既有审稿输入与公开 benchmark policy。`read_roots` 不再指向
整个 `Test2`，因此不会把 `output-final` 或将来加入的真实 SI 声明为可读输入。
host-side verifier、下面的 `numeric_cli.py` 和数值 golden 不进入 bundle。

agent 进程使用环境变量白名单，`inherit_environment=false`，不会继承
`*_TOKEN`、`*_KEY`、`MINERU_API` 等秘密。runner 还通过
`OPENCODE_CONFIG_CONTENT` 注入显式 `external_directory` deny-by-default
规则，只放行 blind bundle 和本次 run 的稳定 Artifact 槽，同时禁止 web
search/fetch、session share 和 subagent；`--auto` 也不能越过显式 deny。
为防止覆盖已验证 Artifact，输出槽已存在时命令会拒绝继续；请换一个新的
`--workfolder`。

live v2 当前仍在运行；本文档不记录其成功状态。完成后应独立报告该 run 的
workflow state、typed run summary、每个 task 的 verifier/repair 轨迹和最终科学
assessment，而不是引用 `output-final` 的离线状态。

也可以只运行独立数值复核：

```powershell
python Test\cases\Test2\reproduction\numeric_cli.py `
  --pdf Test\cases\Test2\Testarticle.pdf `
  --markdown Test\cases\Test2\review-work\Extractedmd\full.md `
  --json-out Test\cases\Test2\reproduction\numeric-output\validation-report.json `
  --markdown-out Test\cases\Test2\reproduction\numeric-output\validation-report.md
```

该 CLI 是 host-side 审计工具，不是给 live agent 的答案模板。live agent 只能读取
blind bundle，并须由正文自行推导；发布前再由 host registry 私下独立重算。host
复核是第三层证据：它能拒绝不满足本地数学合同的候选，但不能在没有 publisher SI
时判定历史表达 identity。

## 验证

```powershell
python -m unittest discover -s Test\cases\Test2\tests -p test_reproduction.py -v
python -m unittest discover -s Test\cases\Test2\tests -v
python -m compileall -q Test\cases\Test2\script Test\cases\Test2\reproduction Test\cases\Test2\tests
```

故障传播测试会分别篡改 numeric、theory、methods、final claim 与 manifest。
JSON 即使仍满足 Schema，只要证据锚点、关键字段、`unknown/BLOCKED` 边界、
数值交叉一致性或稳定输出槽不匹配，领域 verifier 都会拒绝发布，并阻止依赖任务。

## 稳定产物

一次成功运行会发布：

- `Evidence/evidence-report.md` 与 `evidence-ledger.json`；
- `Inference/Theory/theory-inference.{md,json}`；
- `Inference/Methods/methods-inference.{md,json}`；
- `Validation/validation-report.{md,json}`；
- `Report/inferred-supplement.md`；
- `Report/reproduction-assessment.json`；
- `Report/artifact-manifest.json`；
- `9900_run/<run-id>/workflow-result.json` 与 `run-summary.json`。

`artifact-manifest.json` 只接受六个固定发布相对路径，并对对应 Clef
Artifact source 计算 SHA-256；不存在路径、`..` 越界和拿其他槽路径冒用都会
失败。每项 `verification: clef_verified` 仅说明当前本地合同和 digest 通过，
不携带 live autonomy、full reproduction 或 historical identity 含义。

详细的 attempt workspace、验证结果和发布状态保留在 `workflow-result.json` 中。
`run-summary.json` 的 `execution_summary` 保存 SDK 的 workflow/task 终态、起止与
耗时、attempt/repair/verification/publication 聚合计数；它是运行层摘要，不替代
`reproduction-assessment.json` 的科学结论。
