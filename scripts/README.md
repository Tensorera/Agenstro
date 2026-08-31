# Local quality commands

Run the repository gates from the repository root with PowerShell 7:

```powershell
./scripts/quality.ps1 -Profile Fast
./scripts/quality.ps1 -Profile Full
./scripts/quality.ps1 -Profile Release
```

`Fast` is the normal iteration gate. `Full` adds the MSRV, every model-free
test, cross-language acceptance, documentation example compilation, and the
desktop package. `Release` additionally requires a clean worktree and
builds release artifacts; use the manual GitHub workflow for the independent
Windows/Ubuntu compiler matrix.

Dependency refreshes and advisory checks have explicit profiles:

```powershell
./scripts/quality.ps1 -Profile Bootstrap
./scripts/quality.ps1 -Profile Audit
```

Run `Bootstrap` before the warm-cache gates on a new machine. Cargo or Cabal can
still retrieve a missing package archive during `Fast`/`Full`; neither profile
invokes a live model provider. Every run writes command exit codes, tool
versions, logs, and an `agenstro.quality/v1` summary below ignored
`Build/quality/`.

Cargo build output is retained for fast incremental checks. Clean it explicitly
or only after it crosses a chosen threshold:

```powershell
./scripts/quality.ps1 -Profile Clean
./scripts/quality.ps1 -Profile Full -CleanIfOverGiB 5
```

Review `cargo clean --dry-run` directly when diagnosing an unexpected target
directory. The quality script refuses to clean anything outside this checkout's
resolved `Build/cargo` directory. The optional pre-push hook applies the same
5 GiB threshold after a successful Full gate. It requires a clean worktree so
the tested tree is the current `HEAD`, and it rejects pushes of another local
object. The pre-commit hook likewise rejects unstaged or untracked source before
testing the staged snapshot.

Quality runs temporarily pin `CARGO_TARGET_DIR` to that same repository-local
directory and restore the caller's environment afterward, so an inherited
machine-wide target override cannot make the receipt and cleanup inspect a
different tree.
