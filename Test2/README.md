# Test2 稿件审查工作流

本目录实现 `脚本任务.md` 中的完整流程：

1. 纯 Python 调用 MinerU Precision Extract API v4，上传本地 PDF、轮询、下载并
   安全解压；
2. 使用 Clef SDK 并行执行 12 个独立审查 TaskRun；
3. 使用第 13 个 fan-in TaskRun 综合所有已验证审查 Artifact；
4. 保存人读报告、机器读 JSON、workflow result、Artifact manifest 和 trace。

脚本不会修改 `clef-sdk/src/clef_sdk`。MinerU token 只在 OCR 预处理阶段读取，不会
进入 agent 的环境、prompt、Artifact 或日志。

## 运行

从仓库根目录执行：

```powershell
python Test2\script\main.py Test2\Testarticle.pdf `
  --workfolder Test2\review-work
```

如有目标期刊或会议，建议明确提供：

```powershell
python Test2\script\main.py Test2\Testarticle.pdf `
  --workfolder Test2\review-work `
  --venue "Target Journal" `
  --venue-guidelines D:\path\to\author-guidelines.txt
```

常用选项：

- `--plan-only`：完成/复用 OCR 后只编译 13 节点 DAG，不调用 agent；
- `--skip-ocr`：仅复用与 `manuscript.pdf` digest 匹配的 `Extractedmd`；
- `--force-ocr`：成功下载新结果后，只替换当前 workfolder 的 `Extractedmd`；
- `--ocr-model vlm|pipeline`、`--ocr-language en`：MinerU 参数；
- `--profile`：替换 agent/runtime TOML 模板。

OCR 与 agent 执行都可能产生外部费用。默认命令不会清理已有审查输出；稳定输出
槽存在时会拒绝覆盖，请使用新的 workfolder。

## 离线验证

测试不读取真实 `.env`、不联网、不启动 agent：

```powershell
python -m unittest discover -s Test2\tests -v
python -m compileall -q Test2\script Test2\tests
```

审稿规范来自 Nature、Elsevier、COPE、EQUATOR 和 ICMJE 的官方公开资料。本地
`Reviewfunction/references` 保存来源链接和精炼检查表；它们是通用审查参考，只有
`review-context.json` 明确指定目标 venue 时，才可视为 venue 规则。

MinerU API 实现依据：
https://mineru.net/doc/docs/index_en/

## 缺失 Supplementary 盲复现

`reproduction` 子目录把同一论文作为 Clef SDK 的端到端复现基准。它在
不访问真实 supplementary 的情况下，执行一个 `1 -> 3 -> 1` 的五节点 DAG：
盘点缺失证据，分别推导理论与实验/FEA 边界，独立重算正文数值，再综合经过验证的
Artifact。

`reproduction/output-final` 是确定性的 offline reference，用来回归 DAG、
公开输出合同、host verifier、发布和 digest；它不是 live-agent autonomy 的
证据。该 reference 的本地验证结果为 9 项 `PASS`、0 项 `FAIL`、3 项预期
`BLOCKED`：9 项由 2 项输入身份、1 项 SI 范围盘点和 6 项科学计算检查组成。
科学判定是 `partial_reproduction`。三个阻塞分别对应作者历史上的斜压缩函数基
表示、Fig. S1 ABAQUS 全场结果和实验原始数据；框架不会用猜测值把它们伪装成
成功。

需要分别阅读四层状态：

1. Clef workflow 是否 `SUCCEEDED`；
2. Artifact 是否通过当前本地合同、领域 verifier 和 digest；
3. 科学结论是否只是 `partial_reproduction`；
4. 与出版商历史 SI 的表达是否被比对；当前为 **not evaluated**。

live OpenCode 运行和 host-side 独立复核是另外两层证据。正在执行的 live v2
尚无成功结论，不能由 `output-final` 代替。

详见 `Test2/reproduction/README.md`。快速运行：

```powershell
python Test2\reproduction\run.py --mode offline `
  --workfolder Test2\reproduction\output-final
```
