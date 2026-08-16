# Artifact 结构

调用参数中的 `workfolder` 是 Clef SDK 的 workspace root。稳定 Artifact
和运行审计目录如下：

```text
workfolder/
  manuscript.pdf
  review-context.json
  Extractedmd/
    full.md
    images/...
    ...MinerU 其他原始结果...
    extraction-manifest.json
  Reviewprocess/
    01_formal/
      review.md
      findings.json
    02_scope/
      review.md
      findings.json
    ...
    12_ethics/
      review.md
      findings.json
  Report/
    final-review-report.md
    decision.json
    workflow-result.json
    artifact-manifest.json
    run-summary.json
  9900_run/
    run-<id>/
      progress.jsonl
      attempt-workspaces/
        <task-id>/attempt-0001/...
```

Clef SDK 自有状态与 workfolder 保持为不相交的兄弟目录：

```text
workfolder-parent/
  .clef-state/
    <workfolder-name>-<path-digest>/
      cas/
      traces/
      cache/
      manifests/
```

## Artifact 契约

- OCR 不是 agent 任务；`manuscript.pdf`、`Extractedmd/full.md` 和
  `review-context.json` 以有 digest 的直接 `ArtifactRef` 进入 12 个审查节点。
- 每个审查节点只可创建 `review.md` 与 `findings.json`。两者需分别通过最小可见
  文本长度、JSON Schema 和跨文件 issue-id 一致性验证。
- 最终节点通过 24 条 `ArtifactBinding` 接收 12 组已验证输出；任何审查节点失败，
  最终节点都会被跳过。
- 最终输出需通过最小长度、JSON Schema、十二维度覆盖、上游 verdict 对齐和
  issue-id 来源验证后才会发布到 `Report`。
- `RetryWorkspaceStrategy.NEW` 让每次内容重试写入 run-scoped 私有 workspace；
  只有验证成功的 attempt 会原子发布到稳定槽。

`artifact-manifest.json` 按阶段、任务序号和输出序号列出已成功发布的全部 Artifact。
`workflow-result.json` 保留完整 attempt、验证、变更、usage 和 trace 引用。
