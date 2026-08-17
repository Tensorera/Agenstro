# Segno Flow

Segno Flow `0.3.0.0` is the Haskell persistent-task layer for Agenstro. The
public API keeps trigger and business-state types in Haskell; the single-node
driver owns durable cursors, occurrences, attempts, leases, fencing, and
waiting; each actual Clef task still runs through the Rust Tactus kernel.

The removed Python package, Rust `segnod` crates, ZIP task format, and separate
desktop UI are not current compatibility surfaces.

## Small model

```text
Trigger state event + State state + Clef Workflow decision
                         |
                         v
                    PersistentTask
```

- Trigger leaves are open plugins. Haskell composes them with
  `mapTrigger`, `filterTrigger`, `mergeTrigger`, and state-aware `gate`.
- `State state` is typed and versioned. Explicit checkpoints use short
  compare-and-set transactions and never roll back implicitly.
- Lifecycle state is driver-owned and stored separately from business state.
- Business-state and lifecycle commits are not a cross-database exactly-once
  transaction.
- A workflow returns `Ignore`, `Complete`, `Retry`, or `Fail`.
- Delivery is at least once. Ambiguous transport/external outcomes become
  `OutcomeUnknown` rather than an automatic unsafe retry.

The built-in `time.interval` and UTC `time.cron` plugins are pure planners.
They calculate due occurrences and next wake time but never sleep. The driver
owns sleeping and cursor persistence.

## Build, install, or upgrade on Windows

From the repository root, install the current Rust Tactus and Haskell Segno
commands into Cargo's executable directory:

```powershell
$repoRoot = (Resolve-Path D:\src\Agenstro).Path
$toolBin = Join-Path $env:USERPROFILE ".cargo\bin"
$env:PATH = "C:\ghcup\bin;$toolBin;$env:PATH"
Set-Location $repoRoot

cargo install --path tactus-runtime --bin tactus --locked --force
cabal update
cabal build --builddir=Build/cabal all --enable-tests
cabal test --builddir=Build/cabal segno-flow:test:segno-flow-tests --test-show-details=direct
cabal install segno-flow:exe:segno `
  --builddir=Build/cabal `
  --installdir $toolBin `
  --overwrite-policy=always

Get-Command tactus,segno -All
tactus --version
segno --version
tactus check --help | Select-String -Pattern '--package'
```

These are also the upgrade commands. The `--package` help check catches an
older Tactus binary that can share the `0.3.0` version string but cannot expose
the Segno package to GHC. If another executable resolves first, fix `PATH` or
open a new terminal.

The test suite is model-free. Scheduler coverage uses virtual time and fake
process boundaries; it does not wait a real minute or inspect the developer's
foreground application. CI separately type-checks the active-window task.

## Quick start

Set the checkout and target paths. If the target already contains
`.tactus\tactus.toml`, keep it and run only the idempotent `segno init` step:

```powershell
$projectRoot = (Resolve-Path D:\work\my-project).Path
if (-not (Test-Path (Join-Path $projectRoot ".tactus\tactus.toml"))) {
  tactus init $projectRoot --sdk (Join-Path $repoRoot "clef-sdk")
}

tactus doctor --root $projectRoot
segno init --root $projectRoot --sdk (Join-Path $repoRoot "segno-flow")

$script = Join-Path $projectRoot ".tactus\scripts\900_record_active_window.hs"
Copy-Item `
  (Join-Path $repoRoot "segno-flow\examples\active-window\900_record_active_window.hs") `
  $script

# Compile and resolve packages without executing the task.
tactus check --root $projectRoot --package segno-flow `
  --timeout-seconds 7200 $script

segno install --root $projectRoot $script
segno once --root $projectRoot
segno status --root $projectRoot --job record-active-window
segno history --root $projectRoot --state-key example.active-window --limit 20
```

The first check on a fresh Cabal installation may download Hackage packages and
compile Clef, Segno, cron, SQLite bindings, and Win32 support. Several minutes
is normal; later checks reuse the Cabal store and build cache. Direct Tactus
`check/run --timeout-seconds 0` disables its deadline if a finite cold-build
budget is unsuitable.

Keep the scheduler active with:

```powershell
segno driver --root $projectRoot --poll-seconds 5
```

Long-running jobs can set `--task-timeout-seconds` on `install`, `once`, and
`driver`. It defaults to 1,800 seconds for each Tactus build/run phase, accepts
1 through 604,800, and rejects zero. `--poll-seconds` is only the driver's
maximum idle wait; it does not change the 60-second trigger or task timeout.
The driver derives a safe minimum Running lease from the task budget.

The example captures the active window on Windows once every 60 seconds and
makes no model or network call; the first `once` is due immediately. Window
titles can include document names, URLs, or account names. Collection is
explicit, and both `.tactus\segno\state` SQLite history and `.tactus\runs`
evidence retain the titles locally. Review them before committing or sharing.

Delivery is at least once, so external effects should use the occurrence
idempotency key. If a timeout or malformed terminal result leaves execution
ambiguous, Segno records terminal `OutcomeUnknown` and does not retry it
automatically. A successful checkpoint remains durable; inspect local evidence
and reconcile the external system before deciding whether to act again.

## Workspace state

```text
.tactus/segno/
  jobs/                 installed task manifests
  state/
    business.sqlite3   typed workflow state/history
    lifecycle.sqlite3  trigger cursors and runtime lifecycle
  triggers/             bounded invocation/result exchanges
```

`segno init` also registers `time.interval`, `time.cron`, `segno.state`, and
`system.active-window` as one-shot plugins in `.tactus/tactus.toml` and adds
the local `segno-flow` Cabal package to `.tactus/cabal.project`.

## Boundaries

Segno is persistent scheduling, not replay. Version one has no exactly-once
guarantee, distributed driver, serialized Haskell continuation, arbitrary
workflow replay, external-effect rollback, authentication, or network daemon
API.

See the [full Segno guide](../docs/segno.md), [architecture](../docs/architecture.md),
and [ADR-0004](../docs/adr/0004-haskell-segno-persistent-tasks.md).
