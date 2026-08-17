# Agenstro 0.3 Documentation

This site describes the current `0.3` source path: Clef is a typed Haskell
EDSL, Tactus is the Rust execution kernel that initializes projects,
supervises plugins and Haskell programs, streams events, and records factual
run journals, and Segno is the Haskell driver for typed persistent tasks.

## Current release contract

| Surface | Contract |
| --- | --- |
| Clef | Cabal package `0.3.0.0`; GHC2021; `base >=4.20 && <4.23` |
| Tactus | Rust crate/binary `0.3.0`; `init`, `list`, `prompt`, `generate`, `check`, `run`, `doctor`, `smoke`, and `plugin-call` |
| Plugins | Replaceable one-shot JSONL processes; built-in adapters cover Codex, Claude Code, OpenCode, and `workspace.paths` |
| Effect | `workspace.paths` observes path snapshots and differences; it does not sandbox or roll back changes |
| Segno | Cabal package `0.3.0.0`, CLI `0.3.0`; typed triggers and state, pure interval/UTC-cron planning, SQLite, single-node at-least-once driver |
| Motivo Studio | TypeScript + React + Electron `0.3.0`; redacted visual projection over versioned Tactus control queries |
| Platforms | Windows and Ubuntu are required jobs for Tactus, Clef/Segno integration, and Motivo Studio; real active-window capture is Windows-only |

## Fast Windows path

From the Agenstro checkout, install or upgrade both public commands into the
Cargo bin directory already used by Rustup:

```powershell
$toolBin = Join-Path $env:USERPROFILE ".cargo\bin"
$env:PATH = "C:\ghcup\bin;$toolBin;$env:PATH"
cargo install --path tactus-runtime --bin tactus --locked --force
cabal update
cabal install segno-flow:exe:segno `
  --builddir=Build/cabal `
  --installdir $toolBin `
  --overwrite-policy=always
tactus check --help | Select-String -Pattern '--package'
segno --version
```

For a project that already contains `.tactus\tactus.toml`, do not create a
second workspace. Run `segno init --root PROJECT --sdk REPO\segno-flow`, then
follow the [one-minute active-window example](segno.md#try-the-model-free-active-window-task).
The first Cabal build can take several minutes and fetch packages; later runs
reuse its cache. The example makes no model/network call. Foreground-window
titles remain in local SQLite business history; Tactus run evidence retains
only a bounded diagnostic summary.

From the repository root, the source gates are:

```powershell
cabal build --builddir=Build/cabal all --enable-tests
cabal test --builddir=Build/cabal all --test-show-details=direct

$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo check -p tactus-runtime --all-targets --locked
  cargo test -p tactus-runtime --locked
  cargo clippy -p tactus-runtime --all-targets --locked -- -D warnings
} finally {
  cargo clean
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}

npm --prefix motivo-studio ci
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio run typecheck
npm --prefix motivo-studio test
npm --prefix motivo-studio run package
```

Those tests use local fakes and do not contact a model provider. Segno's
scheduler tests use virtual time and fake process boundaries instead of
waiting for a minute; CI type-checks the active-window task without executing
desktop capture. Motivo packaging produces an unsigned application for the
current platform.

## Tactus safety boundary

`tactus check` performs Cabal/GHC compile checks. `tactus run` executes trusted
Haskell programs, and `tactus generate` invokes the selected coding-agent
plugin. Provider and effect commands inherit the user's environment and run
without a shell, but they are still arbitrary local executables rather than a
security boundary.

`tactus smoke` is offline by default: it performs executable/version health
checks but sends no model prompt. A real provider request happens only when
`--live` is supplied. OpenCode is supported at the JSONL adapter boundary, but
its `--auto` mode cannot prove a full approval bypass in the presence of
explicit deny or managed configuration.

## Start here

| Goal | Page |
| --- | --- |
| Install Tactus and run the offline path | [Getting started](getting-started.md) |
| Understand ownership and execution flow | [Architecture](architecture.md) |
| Run a typed persistent task | [Segno persistent tasks](segno.md) |
| Visualize an initialized workspace | [Motivo Studio](motivo-studio.md) |
| Check current platform and component status | [Support matrix](reference/support-matrix.md) |
| Inspect the provider/effect wire contract | [Local plugin protocol v1](reference/plugin-protocol-v1.md) |
| Diagnose toolchain, encoding, or plugin failures | [Troubleshooting](troubleshooting.md) |
| See what is current, exploratory, or historical | [Roadmap](roadmap.md) |
| Understand the Haskell and trusted-plugin decision | [ADR-0003](adr/0003-haskell-dsl-and-local-plugins.md) |
| Understand the Segno driver and state decision | [ADR-0004](adr/0004-haskell-segno-persistent-tasks.md) |
| Move a `0.2` workspace to the new path | [0.2 to Haskell 0.3 migration](migrations/0.2-to-haskell-0.3.md) |

Motivo Studio is current but deliberately thin: it invokes Tactus and never
becomes a second runtime. Segno is persistent scheduling, not recorded-result
replay. Superseded Clef, Tactus, and Segno implementations remain available
through Git history and selected archives; they are not alternate current
runtimes.
