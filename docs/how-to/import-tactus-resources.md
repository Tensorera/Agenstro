# Import resources into a Tactus runtime

> **Legacy compatibility:** this guide applies to detached runtime roots
> created with `tactus init --source --root`. Current Studio startup works
> directly in the selected project and does not need a resource-import step.

Use `tactus init --resource` for ignored, untracked, or external inputs that
must be available in the detached worktree.

## Import a file

```powershell
tactus init `
  --source "D:\VibeWorkspace\Agentro" `
  --root "D:\TactusRuns\report" `
  --resource "D:\Inputs\report.pdf=inputs/report.pdf"
```

## Import a directory

```powershell
tactus init `
  --source "D:\VibeWorkspace\Agentro" `
  --root "D:\TactusRuns\report" `
  --resource "D:\Models\layout=resources/layout-model"
```

Without `=DESTINATION`, Tactus places the resource at
`resources/<source-name>`.

Mutable resources use copies:

```powershell
--resource "D:\Inputs\working.db=inputs/working.db"
```

For a large input that is explicitly immutable for the complete runtime
lifetime, auto-select a protected hardlink when it is at least the configured
threshold and on the same local volume:

```powershell
tactus init `
  --source "D:\VibeWorkspace\Agentro" `
  --root "D:\TactusRuns\report" `
  --immutable-resource "D:\Models\weights.bin=models/weights.bin" `
  --hardlink-min-bytes 67108864
```

Auto mode falls back to an audited copy if a protected hardlink is
unavailable. To require hardlinking and fail instead of copying:

```powershell
--hardlink-resource "D:\Models\weights.bin=models/weights.bin"
```

Hardlinks are accepted only for explicitly immutable ordinary files on a
fixed local Windows volume. Tactus holds share-deny-write/delete handles,
records identities and hashes in
`<runtime>\.tactus\resource-imports\`, and reacquires and verifies those
leases when the runtime is reopened. A hardlink without the live protection
lease is rejected.

Framework control namespaces such as `.tactus`, `.clef-state`, and
`.run-control` are never materialized into the worktree. Exclusion is based on
namespace ownership, not a blanket filename rule, so a legitimate project
`task.json` is not silently dropped.

## Checkpoint selected new paths

Use `--include` for paths that should be eligible for Git checkpointing:

```powershell
tactus init `
  --source "D:\VibeWorkspace\Agentro" `
  --root "D:\TactusRuns\report" `
  --include "outputs/**"
```

Resource snapshots detect later changes but do not preserve overwritten or
deleted bytes. Read [Tactus resource policy](../reference/tactus-resource-policy.md)
before using large or non-Git inputs.

For a current in-place Studio project, reference existing project files
directly. If a particular task needs isolation, create its working directory
and materialize inputs explicitly in the main script. Do not confuse those
task-local copies or links with `.tactus/Artifact/workspace`, which is only
an artifact-history browsing view.
