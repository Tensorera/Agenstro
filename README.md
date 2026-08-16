# Agenstro

Agenstro is an experimental framework for dynamic coding-agent workflows. An
agent writes ordinary Haskell programs, GHC checks the typed connections
between workflow steps, and trusted local plugins perform provider calls and
observable effects. The core deliberately stays small; policy belongs at the
plugin boundary.

The current source-alpha line is `0.3`:

| Surface | Version | Role |
| --- | --- | --- |
| [`clef-sdk`](clef-sdk/) | Haskell/Cabal `0.3.0.0` | Typed workflow EDSL and open plugin calls |
| [`tactus-runtime`](tactus-runtime/) | Rust `0.3.0` | Workspace, process supervision, event routing, run journals, and CLI |
| Local plugins | `agenstro.plugin/v1` | One-shot JSONL processes in any implementation language |
| [`motivo-studio`](motivo-studio/) | TypeScript/Electron `0.3.0` | Visual Tactus workspace, plugin, action, and trace projection |
| [`segno-flow`](segno-flow/) | frozen | Retained scheduling/replay exploration; not a `0.3` release gate |

## The small core

Clef exposes a conventional Haskell API:

- `Workflow a` uses normal sequential `do` notation.
- `Task input output` gives provider calls statically checked inputs and
  outputs.
- `Operation output` represents a configured effect operation.
- `Plugin input output`, `jsonPlugin`, and `rawPlugin` open the same typed
  boundary to arbitrary plugins.
- `parallel` is the explicit concurrency boundary.
- `require`, `requireBecause`, and `attempt` express typed workflow control.
- `EventSink` receives provider/plugin events as each complete JSONL frame
  arrives; events do not pollute the workflow's typed return value.

Provider names, models, reasoning effort, flags, and plugin-specific options are
open runtime values, not closed Haskell enumerations. GHC checks value wiring;
it does not pretend to statically authorize an external agent.

Tactus is the typed Rust execution kernel around Clef. It validates
`.tactus/tactus.toml`, prepares Clef's runtime JSON, launches each plugin as a
one-shot process group, routes events incrementally, and writes factual
`agenstro.trace/v1` journals below `.tactus/runs/`. It is not a daemon and does
not provide authentication, a credential broker, CAS/artifact storage,
checkpoint restore, or rollback.

Motivo Studio is a thin TypeScript + React desktop client over that Rust
kernel. Electron main invokes the installed `tactus` executable with argv
arrays; the sandboxed renderer receives only Zod-validated, redacted DTOs. It
does not parse runtime TOML, read journals directly, start a daemon, or expose a
general shell.

## Install from this checkout

Install Rust with `rustup` and Haskell with GHCup. Clef currently declares
`base >=4.20 && <4.23`; use a GHC in that range and a matching Cabal. At least
one native coding-agent CLI (`codex`, `claude`, or `opencode`) is needed only
for the corresponding live provider call.

From the repository root:

```powershell
cargo install --path tactus-runtime --bin tactus --locked --force
cabal update
cabal build all
tactus --version
```

The current Rust Tactus path has no Python runtime dependency. A third-party
plugin may still be implemented in Python, TypeScript, C#, Rust, Haskell, or any
other language that can obey the JSONL process contract.

## Try it in a project

The executable is `tactus`; `.tactus` is the directory it creates. Typing
`.tactus` by itself does not run initialization.

```powershell
Set-Location D:\work\my-project
tactus init --sdk D:\src\agenstro\clef-sdk
tactus list
tactus doctor
tactus smoke
```

`init` is idempotent: it creates missing files and preserves existing content.

```text
.tactus/
  tactus.toml       typed provider/effect/plugin configuration
  cabal.project     local link to clef-sdk
  PROMPT.md         instructions injected into generation requests
  scripts/          ordinary .hs and .lhs workflow programs
  runs/             append-only event journals and terminal summaries
```

Ask a provider to create a multi-step workflow, inspect it, type-check it, and
then run it:

```powershell
tactus generate --provider codex "Create a typed multi-step workflow for this project."
tactus list
tactus check
tactus run
```

Runnable entries use names such as `010_contract.hs`, `020_implement.hs`, and
`030_review.hs`; Tactus runs them in numeric/path order. Other Haskell files are
helpers and are never run implicitly. Select files explicitly when needed:

```powershell
tactus check .tactus\scripts\020_implement.hs
tactus run --script .tactus\scripts\020_implement.hs -- --workflow-argument
```

`generate` calls a real provider and may edit the workspace. `check` only asks
Cabal/GHC to compile-check sources. `run` executes ordinary trusted Haskell and
can make further provider calls or arbitrary `IO` actions. Review generated
scripts before running them.

For a deterministic four-stage example, see
[`examples/topology-holes`](examples/topology-holes/). It progresses from an
ASCII-grid parser, through component and hole counting, to a reviewed CLI. Its
checked-in Rust oracle uses a temporary Cargo target directory and cleans it
after the test.

## Visualize a Tactus workspace

Motivo Studio requires Node.js 22.12 or newer and the installed `tactus`
executable above:

```powershell
npm --prefix motivo-studio ci
npm --prefix motivo-studio start
```

Open an initialized project, or choose **Initialize folder** in the application.
The Overview, Workflow, Plugins, and Runs views project Tactus health, ordered
Haskell entries, registry availability, bounded action output, and factual
trace events. Generate, Check, Run, and offline Smoke remain Tactus commands;
live Smoke is a separate explicit action that may contact or bill a provider.

Studio uses the versioned `tactus studio inspect` and `tactus studio events`
control queries. They redact command arrays, options, prompt text, and absolute
script paths. Motivo is not a second workflow runtime, editor, terminal,
scheduler, replay engine, or credential manager.

## Configure open plugins

`tactus init` writes a typed TOML configuration. The built-ins use subcommands
of the installed `tactus` binary, so there are no separate provider/effect host
executables to install:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus", "provider-host", "codex"]
model = "provider-specific-model-id"
effort = "high"

[providers.codex.options]
timeout_seconds = 900

[providers."claude-code"]
command = ["tactus", "provider-host", "claude-code"]

[providers.opencode]
command = ["tactus", "provider-host", "opencode"]

[effects."workspace.paths"]
command = ["tactus", "effect-host", "workspace-paths"]
observe_invocations = true

[plugins.calculator]
command = ["calculator-plugin", "--jsonl"]

[plugins.calculator.options]
precision = 12
```

Provider `model`, `effort`, OpenCode `variant`, and nested `options` remain open
extension points. Commands are argv arrays and are started without a shell.
Use `plugin-call` to exercise any registry directly:

```powershell
tactus plugin-call calculator add --namespace plugin --params '{"left":19,"right":23}'
tactus plugin-call workspace.paths describe --namespace effect
```

`tactus smoke` performs offline executable/version probes by default. Add
`--live` only when you intend to make a real provider request:

```powershell
tactus smoke provider:codex
tactus smoke provider:codex --live
```

## Provider trust model

This alpha intentionally treats workflows and plugins as trusted local code.
They inherit the caller's workspace, environment, credentials, network, and
user permissions. There is no Tactus login or authorization layer.

The bundled adapters request the most permissive non-interactive mode exposed
by each native CLI:

- Codex uses `--dangerously-bypass-approvals-and-sandbox`.
- Claude Code uses `--dangerously-skip-permissions`.
- OpenCode uses `--auto` plus an inline `permission=allow` configuration.

OpenCode is not equivalent to the first two: an explicit deny or managed policy
can still win, so its smoke/description output reports that full bypass is not
provable. Native provider login, pricing, model availability, and organization
policy remain the provider CLI's responsibility.

## Streaming and evidence

Every plugin call writes one UTF-8 `agenstro.plugin/v1` request to stdin, closes
stdin, then reads zero or more correlated event lines and exactly one terminal
result. Tactus validates and forwards complete frames incrementally with bounded
transport queues and places the child in a Unix process group or Windows Job
Object. Deadline, cancellation, and protocol failure terminate the owned
group. Windows Job Objects contain the nested process tree; on Unix, a process
that deliberately creates a new session can escape process-group containment,
so plugins remain trusted local code.

Each supervised call records flushed `events.jsonl` entries and an atomically
published `summary.json` in a unique `.tactus/runs/<run-id>/` directory. These
records use `agenstro.trace/v1`. They are local factual evidence, may contain
provider output, and are not a deterministic replay format or rollback log.

The included `workspace.paths` effect records path kind, size, and SHA-256
changes across an invocation. It reports final added, modified, deleted, and
type-changed paths under bounded path/byte/time budgets, excluding common build
trees such as `target`, `node_modules`, `build`, and `dist-newstyle`. It does not
retain file contents, observe reads/transient writes, prove which process made
a concurrent change, publish artifacts, or restore the workspace.

## Repository map

```text
clef-sdk/haskell/       current Haskell EDSL and contract tests
tactus-runtime/src/     current Rust runtime, CLI, adapters, and journal
examples/topology-holes multi-step workflow plus deterministic oracle
docs/                   current docs and selected migration material
archive/                frozen historical source snapshots
crates/, proto/         retained legacy foundation code, outside the 0.3 gate
motivo-studio/          current TypeScript/React Tactus visualizer
segno-flow/             frozen scheduling/replay exploration
```

Private design notes under `secretdoc/`, `.tactus/` target-project state, model
transcripts, and generated build directories do not belong in Git.

## Verify without growing the workspace

Keep Rust artifacts in a unique system temporary directory and clean it even if
a command fails:

```powershell
$targetDir = Join-Path $env:TEMP ("agenstro-target-" + [guid]::NewGuid().ToString("N"))
$env:CARGO_TARGET_DIR = $targetDir
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo fmt --all --check
  cargo check -p tactus-runtime --all-targets --locked
  cargo test -p tactus-runtime --locked
  cargo clippy -p tactus-runtime --all-targets --locked -- -D warnings
} finally {
  cargo clean --target-dir $targetDir
  Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}

cabal build all --enable-tests
cabal test all --test-show-details=direct

npm --prefix motivo-studio ci
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio run typecheck
npm --prefix motivo-studio test
npm --prefix motivo-studio run package

python -m mkdocs build --strict
```

The Rust, Haskell, and Motivo tests use local fakes and make no authenticated
provider request. Electron packaging is unsigned and platform-local. The final
MkDocs command uses Python only as the documentation tool, not as part of
Tactus.

Start with the [getting-started guide](docs/getting-started.md), [architecture
overview](docs/architecture.md), [plugin protocol](docs/reference/plugin-protocol-v1.md),
[support matrix](docs/reference/support-matrix.md), and [troubleshooting
guide](docs/troubleshooting.md).

This repository is currently private. No public release or additional license
grant is implied by access to this alpha checkout.
