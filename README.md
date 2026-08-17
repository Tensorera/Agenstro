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
| [`segno-flow`](segno-flow/) | Haskell/Cabal `0.3.0.0` | Pluginized typed persistent-task driver for Tactus workspaces |

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
- `Trigger state event`, `State state`, and `PersistentTask` form the small
  typed boundary for durable Segno tasks.

Provider names, models, reasoning effort, flags, and plugin-specific options are
open runtime values, not closed Haskell enumerations. GHC checks value wiring;
it does not pretend to statically authorize an external agent.

Tactus is the typed Rust execution kernel around Clef. It validates
`.tactus/tactus.toml`, prepares Clef's runtime JSON, launches each plugin as a
one-shot process group, routes events incrementally, and writes factual
`agenstro.trace/v1` diagnostic journals below `.tactus/runs/`. Their durable
payloads are redacted/summarized and are not replay state. Tactus is not a
daemon and does not provide authentication, a credential broker, CAS/artifact
storage, checkpoint restore, or rollback.

Segno is the optional Haskell persistent-task driver. Haskell composes typed
trigger leaves with `mapTrigger`, `filterTrigger`, `mergeTrigger`, and `gate`;
plugins provide the leaves and state backends. The first built-ins are pure
interval/UTC-cron planners and SQLite business state. Segno owns waiting,
cursors, attempts, leases, fencing, and lifecycle, while every actual Clef job
still executes through Tactus. Delivery is at least once; it is scheduling,
not replay or an exactly-once transaction.

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

On Windows, put the GHCup shims and Cargo's executable directory on the current
PowerShell `PATH`, then install both public commands into the Cargo directory:

```powershell
$repoRoot = (Resolve-Path D:\src\Agenstro).Path
$toolBin = Join-Path $env:USERPROFILE ".cargo\bin"
$env:PATH = "C:\ghcup\bin;$toolBin;$env:PATH"
Set-Location $repoRoot

cargo install --path tactus-runtime --bin tactus --locked --force
cabal update
cabal build --builddir=Build/cabal all --enable-tests
cabal install segno-flow:exe:segno `
  --builddir=Build/cabal `
  --installdir $toolBin `
  --overwrite-policy=always

Get-Command tactus,segno -All
tactus --version
segno --version
tactus check --help | Select-String -Pattern '--package'
```

These are also the upgrade commands: `--force` replaces an older Tactus and
`--overwrite-policy=always` replaces an older Segno. Checking `--package` is
important because an earlier binary can also print `tactus 0.3.0` while lacking
the Segno package-extension option. Open a new terminal if `Get-Command -All`
still resolves a stale executable first.

The current Tactus and Segno paths have no Python runtime dependency. A
third-party plugin may still be implemented in Python, TypeScript, C#, Rust,
Haskell, or any other language that can obey the JSONL process contract.

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

## Run a persistent task

For an existing initialized project, verify the marker and add Segno in place.
`segno init` preserves existing Tactus content while adding its package link,
plugin registrations, and `.tactus\segno` state layout:

```powershell
$repoRoot = (Resolve-Path D:\src\Agenstro).Path
$projectRoot = (Resolve-Path D:\work\my-project).Path
if (-not (Test-Path (Join-Path $projectRoot ".tactus\tactus.toml"))) {
  throw "Run tactus init first"
}

tactus doctor --root $projectRoot
segno init --root $projectRoot --sdk (Join-Path $repoRoot "segno-flow")
```

An installed task binds typed trigger leaves, typed/versioned business state,
and one Clef workflow. The driver persists trigger cursors and lifecycle, then
runs each due occurrence through Tactus:

```powershell
$script = Join-Path $projectRoot ".tactus\scripts\900_record_active_window.hs"
Copy-Item `
  (Join-Path $repoRoot "segno-flow\examples\active-window\900_record_active_window.hs") `
  $script

# Warm the local Clef + Segno Cabal project without executing the task.
tactus check --root $projectRoot --package segno-flow `
  --timeout-seconds 7200 $script

segno install --root $projectRoot $script
segno once --root $projectRoot
segno status --root $projectRoot --job record-active-window
segno history --root $projectRoot --state-key example.active-window --limit 20

# Keep this terminal open for subsequent one-minute occurrences.
segno driver --root $projectRoot --poll-seconds 5
```

The first cold `tactus check` may download Haskell packages and compile Clef,
Segno, `cron`, SQLite bindings, and Win32 support. Several minutes of compiler
output is normal; subsequent checks reuse the Cabal store and build cache.
`--timeout-seconds 0` disables the direct Tactus deadline, while the example
uses a generous finite value above.

`--poll-seconds` controls only the driver's maximum idle wait; it does not
change the task's 60-second interval. For long-running workflows, pass
`--task-timeout-seconds N` to `segno install`, `once`, and `driver`. Its default
is 1,800 seconds, accepted range is 1 through 604,800, and zero is not allowed.
Segno derives its Running lease from the same budget.

The example records the Windows foreground-window title every minute without a
model or network call. The first `once` is due immediately. Titles can include
document names, URLs, or account names; SQLite business-state history retains
them locally, while Tactus persists only redacted diagnostic summaries of
plugin results. Delivery is at least once, but an ambiguous timed-out/failed
task becomes `OutcomeUnknown` and is not retried automatically. See the
[Segno guide](docs/segno.md) before deciding whether any external effect is
safe to repeat.

## Visualize a Tactus workspace

Motivo Studio requires Node.js 22.12 or newer and the installed `tactus`
executable above. On Windows x64, install the desktop application and
command-line launcher from this checkout:

```powershell
npm --prefix motivo-studio ci
npm --prefix motivo-studio run install:windows
```

The installer replaces the per-user application at
`%LOCALAPPDATA%\Programs\MotivoStudio` and adds that exact directory to the user
`PATH`. Open a new terminal to receive the change, then launch the UI with no
argument or open an initialized workspace directly:

```powershell
motivo-studio
motivo-studio 'D:\work\Project with spaces'
```

Close Studio and run the install command again to upgrade it. Close Studio and
use `npm --prefix motivo-studio run uninstall:windows` to remove the recognized
per-user installation and its user-`PATH` entry. Development still uses
`npm --prefix motivo-studio start`; it runs Electron Forge from the checkout
without installing the command.

If Studio is already running, another `motivo-studio [WORKSPACE]` invocation
focuses the existing window and switches it to the supplied workspace. You can
also choose **Open workspace** or **Initialize folder** in the application.

The Overview, Workflow, Plugins, and Runs views project Tactus health, ordered
Haskell entries, registry availability, bounded action output, and factual
trace events. Generate, Check, Run, and offline Smoke remain Tactus commands;
live Smoke is a separate explicit action that may contact or bill a provider.
User-facing action and run logs use only `[state]`, `[info]`, `[warning]`, and
`[error]` plus bounded natural-language messages. Structured payloads, event
kinds, exit codes, and unmatched legacy output remain available under
collapsed technical details instead of appearing as raw JSON by default. If
the desktop action projection reaches its byte or frame budget, Motivo keeps
draining Tactus, omits further raw output, emits one `[warning]`, and preserves
the child process's authoritative terminal status.

Studio uses the versioned `tactus studio inspect` and `tactus studio events`
control queries. They redact command arrays, options, prompt text, and absolute
script paths. Motivo is not a second workflow runtime, editor, terminal,
scheduler, replay engine, or credential manager.

Agents editing a workflow can follow the repository-local
[`tactus` skill](skills/tactus/SKILL.md). It guides the agent to constrain
requested edits to `.tactus/scripts`, validate selected files before execution, and treat
`OutcomeUnknown` as terminal evidence that must be reconciled rather than
blindly retried.

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

Observation and presentation are deliberately weaker than the invocation
result. Native provider progress is aggregated into bounded diagnostics;
callback pressure can drop low-priority events into `events_dropped`, and a UI
projection overrun emits one `[warning]`. A stalled observer is recorded as
`observation_error`. None of these conditions changes the invocation kind or
replaces its authoritative terminal result; transport/protocol violations
remain a separate outcome boundary.

Each supervised call attempts to append/flush `events.jsonl` and atomically
publish `summary.json` in a unique `.tactus/runs/<run-id>/` directory. A
journal-writer failure degrades evidence without replacing a known invocation
outcome. Durable records use `agenstro.trace/v1`; they are diagnostic evidence,
not a deterministic replay format or rollback log. Durable events recursively
redact prompt, raw/text/content, credential-like, options, environment,
workspace, and path fields to bounded byte-count/SHA-256 summaries; terminal
success values and native stderr are also summarized instead of stored
verbatim. Remaining diagnostics are bounded but may still be sensitive, so
journals stay local.

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
motivo-studio/          current TypeScript/React Tactus visualizer
segno-flow/             current Haskell persistent-task driver and plugins
skills/                 bundled agent guidance for working with Tactus
Test/                   repository-level fixtures and contract checks
Build/                  ignored build output, recreated by project tools
```

Private design notes under `secretdoc/`, `.tactus/` target-project state, model
transcripts, and generated `Build/` content do not belong in Git. Removed 0.2
implementations and foundation crates remain available through Git history.

## Verify with centralized build output

Repository configuration directs Cargo to `Build/cargo`, Cabal to
`Build/cabal`, MkDocs to `Build/site`, and Electron Forge to `Build/motivo`.
The directory is ignored and can be removed or recreated at any time:

```powershell
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo fmt --all --check
  cargo check -p tactus-runtime --all-targets --locked
  cargo test -p tactus-runtime --locked
  cargo clippy -p tactus-runtime --all-targets --locked -- -D warnings
} finally {
  cargo clean
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}

cabal build --builddir=Build/cabal all --enable-tests
cabal test --builddir=Build/cabal all --test-show-details=direct

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
overview](docs/architecture.md), [CLI reference](docs/reference/cli-v0.3.md),
[Segno guide](docs/segno.md), [plugin protocol](docs/reference/plugin-protocol-v1.md),
[support matrix](docs/reference/support-matrix.md), and [troubleshooting
guide](docs/troubleshooting.md). Release-level changes are summarized in the
[changelog](CHANGELOG.md).

This repository is currently private. No public release or additional license
grant is implied by access to this alpha checkout.
