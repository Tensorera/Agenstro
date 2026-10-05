# Local quality commands

Run the repository gates from the repository root with PowerShell 7:

```powershell
./scripts/quality.ps1 -Profile Fast
./scripts/quality.ps1 -Profile Full
./scripts/quality.ps1 -Profile Release
```

`Fast` is the normal iteration gate. `Full` adds the MSRV, every model-free
test, cross-language acceptance, documentation example compilation, and the
offline HTML report-template build. `Release` additionally requires a clean worktree and
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

Every quality profile enforces a project cache budget when it finishes,
including after a failed check. The default is **2 GiB (2,147,483,648 bytes,
binary)**; compilation can temporarily exceed it. The budget includes Cargo
and Cabal output, root `dist-newstyle`, generated documentation sites, Vite
caches, and project-local Node/Python tool dependencies. Tool dependencies are
retained and counted; if the budget is reached, rebuildable compilation and
generated-output directories are removed. If retained tool dependencies alone
reach the budget, cleanup reports a failure without deleting them. Quality logs
and summaries remain. Inventory sizes use file lengths rather than filesystem
allocation sizes.

```powershell
./scripts/quality.ps1 -Profile Full
./scripts/quality.ps1 -Profile Clean
./scripts/build-cache.ps1 -MeasureOnly
./scripts/build-cache.ps1
```

`Clean` (or `-Clean` on another profile) forces that cleanup. The standalone
cache script prints a JSON inventory/receipt; `-MeasureOnly` inspects without
writing and `-Force` forces cleanup. `-CacheBudgetBytes` selects a smaller budget.
Available main executables for the current host platform and Cabal source
distributions are copied to fixed
paths under `Build/artifacts/cargo/{debug,release}/`, `Build/artifacts/segno/`,
and `Build/artifacts/cabal/` before removing compiler output. These deliverables
are separate from the cache budget. The scripts restrict deletion to known
project-local rebuildable directories and leave global Cargo/Cabal caches and
toolchains alone.

The shared Cargo dev/test profiles disable debug symbols and compiler
incremental output. Unchanged dependency builds remain reusable until cleanup,
but debugger detail and reuse within a changed crate are reduced. Direct
`cargo`, `cabal`, or documentation-build commands do not run the budget policy;
follow them with `./scripts/build-cache.ps1` when needed.

The optional pre-push hook runs Full with the same default budget. It requires
a clean worktree so the tested tree is the current `HEAD`, and it rejects
pushes of another local object. The pre-commit hook likewise rejects unstaged
or untracked source before testing the staged snapshot.

Quality runs temporarily pin `CARGO_TARGET_DIR` to that same repository-local
directory and restore the caller's environment afterward, so an inherited
machine-wide target override cannot make the receipt and cleanup inspect a
different tree.
