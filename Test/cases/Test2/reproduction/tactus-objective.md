# Tactus / Studio 案例目标：盲复现缺失 Supplementary

把下面的目标作为 Test2 在 Studio 中的 compose 输入。它描述预期结果与边界，
实际可执行计划由 `workflow.py` 构建。

## 目标

仅使用 `Test/cases/Test2/Testarticle.pdf`、固定 OCR 和本地审稿产物，推导这篇论文缺失
supplementary 的可识别内容，并验证推导是否与正文方程、图中数值和物理极限一致。
最终发布人读 supplement、机器读判定和带摘要的 Artifact manifest。

## 强制约束

- 不下载、不搜索、不读取真实 publisher supplementary；
- 不读取 `.env`，不把 MinerU token 传入 agent；
- 每条结论区分为正文事实、数学推导、结构推断或不可识别；
- 不得猜测 ABAQUS 版本、网格、基底本构、实验样本量、误差棒或原始坐标；
- `BLOCKED` 项不得被提升为 `PASS`；
- 数值结果由确定性 verifier 独立重算，任何 `FAIL` 都阻断最终综合。

## 计划形状

先盘点 SI 依赖，再并行执行理论推导、实验/FEA 方法边界推导和数值复核，最后只
消费已验证 Artifact 进行综合：

```text
evidence -> {theory, methods, numerics} -> supplement
```

## 验收

- 编译结果是 5 个 TaskRun、11 条 binding，最大并发度为 3；
- 数值基线为 9 `PASS`、0 `FAIL`、3 `BLOCKED`；
- 总状态是 `SUCCEEDED`，科学结论是 `partial_reproduction`；
- 最终 manifest 包含 evidence、theory、methods、validation、supplement 和
  assessment 的路径及 SHA-256；
- 人为篡改数值 Artifact 时，验证失败且 final fan-in 被跳过；
- discussion 阶段可讨论“完全复现是否可宣称成功”，结束后转 compose；正确答案
  必须保留三项阻塞，而不是通过补写合理参数制造全复现。

## 可执行入口

离线回归：

```powershell
python Test\cases\Test2\reproduction\run.py --mode offline `
  --workfolder Test\cases\Test2\reproduction\output-final
```

真实 agent：

```powershell
python Test\cases\Test2\reproduction\run.py --mode agent `
  --profile Test\cases\Test2\reproduction\reproduction_profile.toml `
  --workfolder Test\cases\Test2\reproduction\agent-output
```

两种模式必须使用相同的 `WorkflowPlan` 与 verifier registry；差异只在 adapter。
