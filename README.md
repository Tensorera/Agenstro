# Agenstro

Agenstro is a typed, local-first system for building coding-agent workflows.
An agent can write ordinary Haskell programs, GHC verifies the value wiring,
Rust supervises the external tools, and every durable task or visual surface
stays behind an explicit runtime boundary.

The current release line is `0.3`.

## Why these names?

The names use a musical metaphor because Agenstro coordinates independent
parts without pretending they are one monolithic agent:

| Name | Naming idea | Function |
| --- | --- | --- |
| **Agenstro** | “agent” + “orchestration” | The whole system: typed composition, execution, persistence, plugins, and visualization |
| **Clef** | A clef gives notes a typed frame of reference | The Haskell EDSL that defines `Workflow`, typed tasks, effects, generic plugins, typed norms/rubrics, and persistent-task values |
| **Tactus** | The measured pulse that turns a score into an execution | The Rust CLI/runtime that owns `.tactus`, selects scripts, supervises processes, stores sessions, routes events, and records diagnostics |
| **Segno** | A score mark that says where execution should continue or return | The Haskell persistent-task driver that owns trigger time, cursors, attempts, leases, and business-state checkpoints |
| **Motivo Studio** | A motif is a recognizable pattern made visible | The TypeScript/React/Electron boundary for redacted projections and typed human decisions |

The metaphor describes responsibility, not hidden coupling. Clef and Segno are
Haskell packages, Tactus is one Rust executable, Motivo is a thin desktop
client, and plugins may be implemented in any language that obeys
`agenstro.plugin/v1`.

## Architecture at a glance

```text
Haskell workflow (Clef)
        |
        v
Tactus Rust runtime ----> one-shot provider/effect/plugin processes
        |
        +----> diagnostic run journal
        +----> durable session store
        +----> Motivo Studio projection + decision return

Segno Haskell driver ----> Tactus ----> one Clef persistent-task occurrence
```

| Component | Version | Status | Owns |
| --- | --- | --- | --- |
| [`clef-sdk`](clef-sdk/) | Haskell `0.3.0.0` | Current | Typed workflow composition, norms/rubrics, and open plugin calls |
| [`tactus-runtime`](tactus-runtime/) | Rust `0.3.0` | Current | Workspace, CLI, process supervision, sessions, event routing, and journals |
| [`segno-flow`](segno-flow/) | Haskell `0.3.0.0` | **Experimental** | Single-node persistent scheduling and versioned state |
| [`motivo-studio`](motivo-studio/) | TypeScript/Electron `0.3.0` | **Experimental** | Redacted projection and typed session answers over Tactus |
| Local plugins | `agenstro.plugin/v1` | Open protocol | Replaceable provider, effect, trigger, or state capabilities |

## Quick installation

The shortest supported source installation needs:

- stable Rust and Cargo;
- GHC/Cabal from GHCup (`base >=4.20 && <4.23`); and
- this repository checkout.

On Windows PowerShell, replace `D:\src\Agenstro` with the checkout path:

```powershell
$repoRoot = (Resolve-Path D:\src\Agenstro).Path
$toolBin = Join-Path $env:USERPROFILE ".cargo\bin"
$env:PATH = "C:\ghcup\bin;$toolBin;$env:PATH"
Set-Location $repoRoot

cargo install --path tactus-runtime --bin tactus --locked --force
tactus --version
tactus check --help | Select-String -Pattern '--package'
```

Create or open a project and initialize it:

```powershell
$projectRoot = "D:\work\my-project"
New-Item -ItemType Directory -Force $projectRoot | Out-Null
Set-Location $projectRoot

tactus init --sdk (Join-Path $repoRoot "clef-sdk")
tactus doctor
tactus list
tactus smoke
```

`tactus init` is idempotent. It creates `.tactus`; typing `.tactus` by itself
does not run initialization. Plain `smoke` is offline and does not send a model
request.

Ask a configured provider to create numbered workflow scripts, inspect them,
type-check them, and run them:

```powershell
tactus generate --provider codex "Create a typed multi-step workflow for this project."
tactus list
tactus check --all
tactus run --all
```

`generate` can contact or bill the selected provider. `check` compiles without
executing workflow code. `run` executes trusted Haskell and any effects it
calls with the current user's operating-system authority. Both commands require
an explicit script selection: paths, `--all`, or an inclusive `--from` /
`--through` entry range.

Inspect recent evidence without opening journal files by hand:

```powershell
tactus runs summarize --since 24h
tactus runs list --state outcome_unknown
tactus runs unfinished
```

For the complete Windows and Ubuntu source-install procedure, upgrades,
optional Segno installation, and Motivo deployment, use the
[installation guide](docs/install.md). For the first controlled workflow, use
the [first-workflow tutorial](docs/getting-started.md).

## Recommended ways to work

The primary recommended workflow is agent-driven:

1. Open a terminal in the project directory that contains `.tactus`.
2. Start your coding agent from that directory so the project is its working
   directory.
3. Ask the agent to use the Tactus skill at
   `.tactus/skills/tactus/SKILL.md`. The skill explains how to inspect,
   generate, edit, check, and run the numbered workflow scripts without
   guessing Tactus commands or workspace boundaries.

The optional graphical workflow is to open the same initialized project in
Motivo Studio:

```powershell
motivo-studio 'D:\work\my-project'
```

Motivo Studio is **experimental**. It is a visual client for Tactus, not a
replacement runtime; use the Tactus CLI as the authoritative fallback when
diagnosing or operating a workspace.

## Optional persistent tasks — Experimental

Install Segno when a typed task must survive between processes and react to
time or another trigger:

```powershell
Set-Location $repoRoot
cabal update
cabal build --builddir=Build/cabal all --enable-tests
cabal install segno-flow:exe:segno `
  --builddir=Build/cabal `
  --installdir $toolBin `
  --overwrite-policy=always

segno --version
```

Segno Flow is **experimental**. It is currently single-node and at-least-once.
It distinguishes scheduler lifecycle state from user business state and never
claims exactly-once external effects. Continue with the
[Segno guide](docs/segno.md).

## Optional desktop view — Experimental

Motivo Studio currently has a Windows x64 per-user installer. It requires
Node.js 22.12 or newer to build from the checkout and an installed `tactus`:

```powershell
Set-Location $repoRoot
npm --prefix motivo-studio ci
npm --prefix motivo-studio run install:windows
motivo-studio 'D:\work\my-project'
```

Motivo is not a second runtime or a general shell. It invokes Tactus, shows
versioned redacted projections, and returns a bounded choice for a pending
session brief. See the [Motivo Studio guide](docs/motivo-studio.md).

## Documentation

Choose the path that matches the work:

### New users

- [Install Agenstro](docs/install.md)
- [Build and run the first workflow](docs/getting-started.md)
- [Configure coding-agent providers](docs/providers.md)
- [Use Motivo Studio](docs/motivo-studio.md)

### Workflow and plugin developers

- [Program with Clef](docs/clef.md)
- [Understand the Tactus workspace and configuration](docs/tactus-workspace.md)
- [Author a local plugin](docs/plugin-authoring.md)
- [Build persistent tasks with Segno](docs/segno.md)
- [Read the plugin protocol reference](docs/reference/plugin-protocol-v1.md)

### Operators and maintainers

- [Understand logs, transitions, and run evidence](docs/observability.md)
- [Operate, back up, and upgrade a workspace](docs/operations.md)
- [Troubleshoot failures](docs/troubleshooting.md)
- [Check supported platforms and boundaries](docs/reference/support-matrix.md)
- [Read the architecture](docs/architecture.md)

The documentation site is built with:

```powershell
mkdocs build --strict
```

## Safety boundary

Workflow programs, configured plugins, and native coding-agent CLIs run with
the authority of the user who starts Tactus. Agenstro validates types,
configuration, protocol frames, and selected process behavior; it is not a
sandbox, credential broker, authorization service, backup system, or rollback
engine. `OutcomeUnknown` deliberately means an external action may have
happened without a trustworthy terminal result.

Read [SECURITY.md](SECURITY.md) before running untrusted workflows or plugins.

## Contributing and verification

See [CONTRIBUTING.md](CONTRIBUTING.md) for source gates and ownership. The
repository keeps generated Rust, Cabal, MkDocs, and Electron output below
`Build/` or ignored tool directories so it can be rebuilt rather than
committed. Release-level changes are recorded in [CHANGELOG.md](CHANGELOG.md).

The canonical model-free local gates are:

```powershell
./scripts/quality.ps1 -Profile Fast
./scripts/quality.ps1 -Profile Full
```

Use `./scripts/quality.ps1 -Profile Clean` when the shared Cargo target becomes
too large, or pass `-CleanIfOverGiB 5` to clean only after a threshold.

## License

Agenstro is licensed under the
[GNU Affero General Public License v3.0 only](LICENSE), identified by the SPDX
expression `AGPL-3.0-only`. If you modify the program and provide its
functionality to users over a network, AGPL section 13 requires offering those
users the corresponding source as described by the license.
