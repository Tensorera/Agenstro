# Agenstro

Agenstro is an experimental, Haskell-first framework for dynamic coding-agent
workflows. Its core idea is deliberately small: an agent writes ordinary
Haskell programs, GHC checks the typed connections between workflow steps, and
replaceable local plugins perform provider calls and observable effects.

The current release line is `0.3` and is an alpha. The supported path is the
Clef Haskell EDSL plus the Tactus Python CLI. The older Rust/Python runtime,
Motivo Studio, and Segno Flow remain in this private repository as migration
material; they are not part of the `0.3` release gate.

| Surface | Version | Role in `0.3` |
| --- | --- | --- |
| [`clef-sdk`](clef-sdk/) | Cabal `0.3.0.0` | Typed Haskell workflow EDSL |
| [`tactus-runtime`](tactus-runtime/) | Python `0.3.0` | Workspace setup, script generation, checking, execution, diagnostics, and plugin smoke probes |
| Local plugins | `agenstro.plugin/v1` | One-shot JSONL adapters for Codex, Claude Code, OpenCode, and `workspace.paths` |
| [`motivo-studio`](motivo-studio/) | frozen | Legacy visual client; not a release gate |
| [`segno-flow`](segno-flow/) | frozen | Legacy scheduler/replay exploration; not a release gate |

## What the workflow model does

Clef exposes a small set of typed building blocks:

- `Workflow a` provides normal sequential `do` notation.
- `Task input output` makes the value flowing into and out of a provider call
  visible to GHC.
- `Operation output` represents a dynamically configured effect operation.
- `parallel` is the explicit concurrency boundary.
- `require` and `requireBecause` stop a workflow when a typed predicate fails.
- `attempt` catches workflow failures without swallowing asynchronous
  cancellation.

Provider names, models, reasoning effort, command-line flags, credentials, and
effect policy are intentionally not closed Haskell enums. They remain open
runtime configuration so that an adapter can evolve independently. Typed
wrappers can be added at the edge without making the core a provider-specific
framework.

## Try it in a project

The command is `tactus`; `.tactus` is the project-local directory it creates.
Typing `.tactus` by itself does not initialize anything.

### 1. Install the toolchain

You need:

- Python `3.12`
- GHC `9.10` through `9.14` and Cabal (GHCup is the easiest installer)
- at least one supported coding-agent CLI: `codex`, `claude`, or `opencode`
- that provider's normal local login or credentials

From this repository on PowerShell:

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install --upgrade pip
python -m pip install -e ".\tactus-runtime"

cabal update
cabal build all
tactus --version
```

Keep the virtual environment active when using `tactus`. For a developer
installation with the test and lint tools, install
`-e ".\tactus-runtime[dev]"` instead.

### 2. Initialize the target project

Change to the project the agent will work on and point Tactus at this checkout's
Clef package:

```powershell
Set-Location D:\work\my-project
tactus init --sdk D:\src\agenstro\clef-sdk
tactus doctor
tactus smoke
```

`init` is idempotent: it creates missing files but does not replace existing
ones. It produces:

```text
.tactus/
  tactus.toml       # provider/effect commands and open options
  cabal.project     # points Cabal at clef-sdk
  PROMPT.md         # instructions injected into generation requests
  scripts/          # ordinary .hs and .lhs workflow programs
```

On Windows installations that still use a legacy console code page, set these
before invoking an older Tactus build if Chinese text or emoji causes a UTF-8
transport error:

```powershell
$env:PYTHONUTF8 = "1"
$env:PYTHONIOENCODING = "utf-8"
```

The `0.3` reference hosts force UTF-8 at their JSONL boundary; the variables
remain a useful compatibility fallback for older editable installations and
third-party Python plugins.

### 3. Generate, inspect, check, and run

```powershell
tactus generate --provider codex "Inspect this project and create a typed workflow that implements and reviews the requested change."
tactus list
tactus check
tactus run
```

Generation asks the selected coding agent to write one or more programs under
`.tactus/scripts/`. Runnable entries use names such as `010_plan.hs`,
`020_implement.hs`, and `030_review.hs`; Tactus executes them in numeric order.
Other Haskell files are helper modules and are never run implicitly.

`generate` does not execute the generated scripts. Read them first, then use
`check` for Cabal/GHC static checking and `run` for execution. An explicit
script may be selected with, for example:

```powershell
tactus check .tactus\scripts\020_implement.hs
tactus run .tactus\scripts\020_implement.hs
```

These programs and plugins are trusted local code. They inherit the current
working directory, environment, provider credentials, and terminal access.
This alpha does not provide an authorization boundary, rollback, or a sandbox.

## Configure providers and effects

`tactus init` writes `.tactus/tactus.toml`. The default provider can be changed,
and model/effort values remain open strings:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus-provider-host", "codex"]
# model = "provider-specific-model-id"
effort = "high"

[providers.codex.options]
timeout_seconds = 900

[providers."claude-code"]
command = ["tactus-provider-host", "claude-code"]

[providers.opencode]
command = ["tactus-provider-host", "opencode"]

[effects."workspace.paths"]
command = ["tactus-effect-host", "workspace-paths"]
observe_invocations = true
```

All command values are argument arrays and are executed without shell parsing.
Adapters may accept open `options`, including `extra_args` and `extra_env`.
Run `tactus smoke` for executable/version checks and use `tactus smoke
codex --live` only when you intentionally want a real, billable provider
request.

The built-in adapters request the most permissive mode their provider exposes.
Codex and Claude Code have explicit dangerous bypass flags. OpenCode's `--auto`
plus inline permission configuration cannot override every explicit deny or
managed policy, so Agenstro does not claim an absolute bypass for OpenCode.

## Plugin boundary

Every provider or effect call launches one process, writes one
`agenstro.plugin/v1` request to standard input, and reads JSONL events followed
by exactly one correlated terminal result. A minimal request is:

```json
{"api":"agenstro.plugin/v1","id":"clef-1","method":"invoke","params":{}}
```

This boundary is language-neutral. A plugin may be written in Haskell, Python,
Rust, TypeScript, C#, or any language capable of reading and writing JSONL.
Provider-specific output is normalized by its adapter; the Haskell core only
knows about the generic protocol and typed wrapper chosen by the workflow.

The included `workspace.paths` effect observes before/after path metadata and
content hashes. It does not capture arbitrary reads, preserve file contents,
attribute concurrent external mutations with certainty, or roll changes back.

## Repository map

```text
clef-sdk/haskell/       current typed EDSL and tests
tactus-runtime/         current workspace CLI and reference plugin hosts
docs/                   current documentation and migration guidance
archive/                frozen 0.2 Python/Tactus source snapshots
crates/, proto/         frozen 0.2 Rust/protobuf foundation
motivo-studio/          frozen visual client
segno-flow/             frozen scheduling/replay exploration
PelicanRide/, Test2/    retained integration and reproduction cases
```

Generated builds, runtime state, private design notes under `secretdoc/`, and
internal model transcripts are deliberately excluded from Git.

## Verify the release gate

From the repository root:

```powershell
cabal build all --enable-tests
cabal test all --test-show-details=direct

python -m pip install -e ".\tactus-runtime[dev]"
python -m pytest tactus-runtime/tests
python -m ruff check tactus-runtime/src/tactus_runtime tactus-runtime/tests
python -m ruff format --check tactus-runtime/src/tactus_runtime tactus-runtime/tests
python -m pyright -p tactus-runtime/pyproject.toml

python -m pip install "mkdocs>=1.6,<2"
python -m mkdocs build --strict
```

The automated Haskell and Python tests use fake local plugins/provider CLIs and
do not make authenticated model requests. Dependency installation can still
contact Hackage or PyPI.

## Current limitations

- This is a source alpha, not a stable package release.
- Plugin output is currently collected per process; a long provider call may
  appear quiet until it exits.
- Arbitrary `liftIO` is allowed and cannot be observed or replayed reliably.
- Path observation reports workspace deltas, not a security audit trail.
- Motivo Studio and Segno Flow are intentionally frozen while the Haskell/Tactus
  contract settles.
- Reliable replay is limited to future plugin-mediated traces; arbitrary
  Haskell IO cannot be replayed deterministically.

Start with the [documentation home](docs/index.md), [getting-started
guide](docs/getting-started.md), [architecture overview](docs/architecture.md),
[plugin protocol](docs/reference/plugin-protocol-v1.md), [support
matrix](docs/reference/support-matrix.md), and [migration
guide](docs/migrations/0.2-to-haskell-0.3.md).

This repository is currently private. No public release or additional license
grant is implied by access to this alpha checkout.
