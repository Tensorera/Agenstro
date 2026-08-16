# Diagnose a Tactus runtime

Use `doctor` before the first compose and when a runtime refuses to advance.
Doctor is read-only with respect to workflow state.

```powershell
tactus doctor `
  --root "D:\Projects\my-project" `
  --agent codex
```

Select `opencode` when that is the compose backend. Use `--executable PATH`
only to test a specific launcher path.

Doctor checks:

- Windows and CPython versions.
- Exact local project path, workspace mode, and optional Git boundary.
- SQLite integrity, schema, owners, and state relationships.
- Direct ipykernel kernelspec and Tactus provisioner registration.
- pywin32 and Job Object support.
- The selected agent launcher.

Doctor does not start a kernel, invoke an agent, migrate storage, recover a
cell, or advance workflow state. A read-only SQLite WAL connection may still
use existing `-wal` and `-shm` sidecar files.

Exit code `0` means every check is healthy. Exit code `1` means at least one
check failed. Use [Support matrix](../reference/support-matrix.md) and
[Tactus CLI reference](../reference/tactus-cli.md) to resolve the reported
boundary.

For a current project, `--root` is the same directory from which Studio was
started. A detached-worktree check is expected only when doctor reads a legacy
`workspace_mode = "detached"` configuration.
