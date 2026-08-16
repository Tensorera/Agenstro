---
title: Getting started with Agenstro 0.3
status: alpha
last_verified: 2026-08-15
applies_to: "Clef Haskell 0.3.0.0 and Tactus Rust 0.3.0"
---

# Getting started with Agenstro 0.3

This guide installs the Rust Tactus CLI, initializes a disposable project,
checks an offline Haskell workflow, and then shows the separate opt-in path for
a real coding-agent provider.

Motivo Studio is an optional visual client for the same Tactus workspace. Segno
Flow remains frozen and is not needed for this path.

## Prerequisites

| Tool | Supported purpose |
| --- | --- |
| Rust stable toolchain | Build/install Tactus `0.3.0` |
| GHC with `base >=4.20 && <4.23` | Compile Clef and workflow programs |
| Cabal | Resolve/build the local Clef package |
| Git | Work with the checkout and target project |
| `codex`, `claude`, or `opencode` | Optional; only for that adapter's provider calls |
| Node.js >=22.12 | Optional; develop or launch Motivo Studio |

Rustup and GHCup are the simplest toolchain installers. Confirm that the
executables are visible in the same terminal:

```powershell
rustc --version
cargo --version
ghc --version
cabal --version
```

Tactus and its built-in adapters do not require Python. MkDocs uses Python only
when building this documentation site, and an arbitrary third-party plugin may
choose Python as its own implementation language.

## Install Tactus and build Clef

From the Agenstro checkout:

```powershell
cargo install --path tactus-runtime --bin tactus --locked --force
cabal update
cabal build all
tactus --version
```

The expected CLI version is `0.3.0`. Installation produces one public
executable, `tactus`; built-in provider/effect adapters are internal
subcommands of that binary.

## Create a disposable project

The following PowerShell example keeps the target separate from the source
checkout:

```powershell
$repoRoot = (Resolve-Path .).Path
$demoRoot = Join-Path $env:TEMP "agenstro-first-run"
New-Item -ItemType Directory -Force $demoRoot | Out-Null
Set-Location $demoRoot
tactus init --sdk (Join-Path $repoRoot "clef-sdk")
```

`init` is idempotent. It writes only missing paths and reports each path as
created or preserved:

```text
.tactus/
  tactus.toml
  cabal.project
  PROMPT.md
  scripts/
  runs/
```

The important distinction is:

- `tactus` is the command;
- `.tactus` is project-local state and configuration;
- running `.tactus` by itself is not initialization.

## Inspect the initialized workspace

```powershell
tactus list
tactus prompt
tactus doctor
tactus smoke
```

`list` initially reports no entries. `prompt` prints the instructions that a
future `generate` call will prepend. `doctor` checks the workspace, toolchain,
SDK linkage, typed configuration, and plugin commands.

`smoke` calls every configured provider, effect, and generic plugin with
`live=false`. For the built-in providers this resolves the native executable
and reads its version; it does not send a model prompt. If an optional provider
CLI is not installed, select only the plugins available on this machine:

```powershell
tactus smoke effect:workspace.paths
tactus smoke provider:codex
```

## Open the optional visual client

After the workspace is initialized, start Motivo from the Agenstro checkout:

```powershell
npm --prefix $repoRoot\motivo-studio ci
npm --prefix $repoRoot\motivo-studio start
```

Choose **Open workspace** and select `$demoRoot`. Motivo projects the same
doctor checks, ordered scripts, registries, and invocation traces through
versioned Rust control queries. It does not read the config or trace directory
itself. See [Motivo Studio](motivo-studio.md) for its actions and boundaries.

## Check and run an offline workflow

Create `.tactus/scripts/010_offline.hs` with this ordinary Haskell program:

```haskell
module Main (main) where

import Clef

main :: IO ()
main = do
  result <- runTactus $ do
    value <- pure (20 :: Int)
    requireBecause "arithmetic invariant" (value + 22 == 42)
    pure (value + 22)
  print result
```

Then inspect, type-check, and run it:

```powershell
tactus list
tactus check
tactus run
```

This verifies the local path from Rust Tactus through Cabal/GHC to Clef without
credentials or a network provider. `check` performs static compilation only;
`run` executes the selected Haskell program.

Explicit selection uses positional paths for `check` and repeatable `--script`
for `run`:

```powershell
tactus check .tactus\scripts\010_offline.hs
tactus run --script .tactus\scripts\010_offline.hs
```

## Understand the initialized plugin configuration

The default `.tactus/tactus.toml` contains three provider adapters, one
observational effect, and an empty generic registry:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus", "provider-host", "codex"]

[providers."claude-code"]
command = ["tactus", "provider-host", "claude-code"]

[providers.opencode]
command = ["tactus", "provider-host", "opencode"]

[effects."workspace.paths"]
command = ["tactus", "effect-host", "workspace-paths"]
observe_invocations = true

[plugins]
```

Tactus decodes category-specific fields into Rust structures while preserving
open nested `options`. For example:

```toml
[providers.codex]
command = ["tactus", "provider-host", "codex"]
model = "provider-specific-model"
effort = "high"

[providers.codex.options]
extra_args = ["--some-new-provider-flag"]

[plugins.calculator]
command = ["calculator-plugin", "--jsonl"]

[plugins.calculator.options]
precision = 12
```

Call any configured registry without writing a Haskell wrapper:

```powershell
tactus plugin-call calculator describe --namespace plugin
tactus plugin-call calculator add --namespace plugin --params '{"left":19,"right":23}'
```

If a name exists in more than one registry, `--namespace` is required.

## Generate a real multi-step workflow

Generation is the first step in this guide that intentionally contacts a
provider and permits that provider to edit the target workspace. Authenticate
the native CLI using its own documented mechanism, then run:

```powershell
tactus generate --provider codex `
  "Inspect this project and create numbered Haskell workflow scripts, from atomic analysis to implementation and review."
tactus list
tactus check
```

`generate` combines `.tactus/PROMPT.md` with the goal and calls the selected
provider. The default instructions require increasing `NNN_slug.hs`/`.lhs`
names below `.tactus/scripts/`. Tactus discovers the resulting files but never
runs them automatically.

Review every generated program before:

```powershell
tactus run
```

The bundled adapters deliberately request high-authority, non-interactive
provider modes:

- Codex: `--dangerously-bypass-approvals-and-sandbox`;
- Claude Code: `--dangerously-skip-permissions`;
- OpenCode: `--auto` with inline `permission=allow`.

OpenCode can still be constrained by explicit deny or managed configuration;
full approval bypass is not guaranteed. Provider authentication, billing,
models, and organization policy remain outside Tactus.

## Follow the topology example

The repository's
[four-stage topology-holes example](https://github.com/Tensorera/agenstro/tree/main/examples/topology-holes)
shows the intended progression:

1. define the grid contract and parser;
2. implement atomic foreground-component counting;
3. add dual-connectivity hole counting and Euler characteristic;
4. review and integrate a complete CLI.

Copy `examples/topology-holes/workflow/*.hs` into `.tactus/scripts/`, or pass
their paths explicitly to `tactus check`. Running those four workflow entries
does make configured provider calls. The separate `reference/` Rust program is
an offline deterministic acceptance oracle.

Keep its compilation outside the checkout and clean it afterward:

```powershell
Set-Location $repoRoot
$targetDir = Join-Path $env:TEMP ("agenstro-topology-" + [guid]::NewGuid().ToString("N"))
$env:CARGO_TARGET_DIR = $targetDir
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo test --manifest-path examples/topology-holes/reference/Cargo.toml --locked
} finally {
  cargo clean --manifest-path examples/topology-holes/reference/Cargo.toml --target-dir $targetDir
  Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}
```

## Read run evidence

Plugin calls and generation produce unique directories below `.tactus/runs/`.
`events.jsonl` is append-flushed as frames arrive; `summary.json` is published
atomically when supervision finishes. Both use `agenstro.trace/v1` envelopes.

These files can contain prompts and provider output. Keep them local and treat
them as diagnostic evidence, not a replay or rollback mechanism.

## Network and trust summary

| Command | Provider network call | Important behavior |
| --- | --- | --- |
| `init`, `list`, `prompt`, `doctor` | No | Local workspace/tool inspection |
| `check` | No provider call | Cabal may fetch missing Haskell packages |
| `smoke NAME` | No model prompt | Native executable/version probe |
| `smoke NAME --live` | Yes for a provider | Minimal real request |
| `generate` | Yes | Provider may modify the workspace |
| `run` | Depends on the program | Executes arbitrary trusted Haskell and plugins |
| `plugin-call` | Depends on plugin/method | Directly executes the configured plugin |

Agenstro `0.3` supplies no daemon, authentication layer, sandbox, CAS, artifact
tracker, checkpoint, or rollback. Use a disposable workspace when you do not
trust generated code or external plugins.
