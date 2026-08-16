---
title: Getting started with Agenstro 0.3
status: alpha
last_verified: 2026-08-15
applies_to: "Clef 0.3.0.0 and Tactus 0.3.0"
platforms: [windows, ubuntu]
---

# Getting started with Agenstro 0.3

This guide exercises the current release path: a Haskell workflow using Clef,
managed by the Tactus Python CLI. It starts with a completely offline workflow
and makes every operation that can contact a model provider explicit.

Motivo Studio, Segno Flow, the old Clef Python package, and the old Tactus
worker/Jupyter path are not part of this guide. They remain frozen migration
evidence and are not substitutes for the 0.3 path.

## 1. Prerequisites

Install the following tools and make them available on `PATH`:

| Tool | Supported baseline | Why it is needed |
| --- | --- | --- |
| Git | A current client | Clone and inspect the source checkout |
| GHC | A release whose `base` is `>=4.20 && <4.23` | Compile Clef and workflow scripts |
| Cabal | A version compatible with that GHC | Build the `clef-sdk` package and run scripts in its package environment |
| Python | CPython `3.12.x` | Run Tactus and its reference plugins |
| Codex, Claude Code, or OpenCode | Optional for the offline path | Required only for the selected provider's smoke or invocation path |

The cross-platform CI baseline uses GHC `9.10.3` and Cabal `3.16.1.0` on
Windows and Ubuntu. Ubuntu also exercises GHC `9.14.1`. Check the installed
tools before continuing:

```powershell
ghc --version
cabal --version
py -3.12 --version
```

On Ubuntu, replace `py -3.12` with `python3.12`.

### Windows UTF-8 compatibility note

The plugin wire protocol is UTF-8. The `0.3` reference provider and effect
hosts reconfigure their protocol standard streams to UTF-8 even when Python
3.12 starts under a legacy Windows code page such as GBK/CP936. This behavior
has a real subprocess regression test.

An older editable Tactus checkout or a third-party Python plugin may not yet do
that. As a compatibility fallback, set these variables in the same PowerShell
session **before** starting the older or custom process:

```powershell
$env:PYTHONUTF8 = "1"
$env:PYTHONIOENCODING = "utf-8"
```

They prevent non-ASCII prompts, paths, provider output, and JSONL frames from
being decoded with the active Windows code page. They are not required by the
current reference hosts and do not change the protocol. See
[Troubleshooting](troubleshooting.md#windows-reports-invalid-utf-8-mojibake-or-a-unicode-error)
for a diagnostic command.

## 2. Install Tactus from the checkout

Run these commands from the Agenstro repository root.

### Windows PowerShell

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\python.exe -m pip install -e ".\tactus-runtime"
$env:PATH = (Resolve-Path .\.venv\Scripts).Path + ";" + $env:PATH

tactus --version
```

### Ubuntu

```bash
python3.12 -m venv .venv
. .venv/bin/activate
python -m pip install -e "./tactus-runtime"
tactus --version
```

The expected Tactus version is `0.3.0`. Installing Tactus also installs the
`tactus-provider-host` and `tactus-effect-host` console entry points. It does
not install GHC, Cabal, or any provider CLI.

## 3. Initialize an isolated project

Do not reuse a 0.2 `.tactus` directory for this walkthrough. Tactus 0.3 creates
only missing files and deliberately preserves existing content, so a clean
project makes the boundary visible.

From the repository root in PowerShell:

```powershell
$repoRoot = (Resolve-Path .).Path
$demoRoot = Join-Path $env:TEMP `
  ("agenstro-quickstart-" + [guid]::NewGuid().ToString("N"))

tactus init $demoRoot --sdk (Join-Path $repoRoot "clef-sdk")
Set-Location $demoRoot
```

On Ubuntu:

```bash
repo_root=$(pwd)
demo_root=$(mktemp -d -t agenstro-quickstart-XXXXXX)
tactus init "$demo_root" --sdk "$repo_root/clef-sdk"
cd "$demo_root"
```

Initialization creates this project-local layout:

```text
.tactus/
  tactus.toml       provider and effect command arrays
  cabal.project     path to the Clef Cabal package
  PROMPT.md         instructions prepended by `tactus generate`
  scripts/          Haskell entry programs and helper modules
```

`tactus init` never replaces an existing file. Its `created` and `preserved`
rows are therefore meaningful; inspect preserved files before relying on them.

## 4. Run an offline Haskell workflow

Create `.tactus/scripts/010_hello.hs` with the following content:

```haskell
import Clef

main :: IO ()
main = runTactus (liftIO (putStrLn "TACTUS_OFFLINE_OK"))
```

The three-digit prefix makes the file a runnable entry. Files without the
`NNN_slug.hs` or `NNN_slug.lhs` convention are treated as helper modules unless
selected explicitly.

Inspect and validate the project:

```powershell
tactus list
tactus doctor
tactus check
tactus run
```

The first `check` or `run` may take longer while Cabal builds `clef-sdk`.
`check` builds Clef and asks GHC to type-check selected scripts with
`-fno-code`; it does not execute the workflow. `run` executes numbered entries
in numeric order with `runghc`. The final command should print:

```text
TACTUS_OFFLINE_OK
```

This example uses `liftIO` and does not invoke a provider or effect. It proves
the Python CLI -> Cabal -> GHC/runghc -> Clef path without credentials or a
model request.

## 5. Understand the default plugins

The generated `.tactus/tactus.toml` registers:

| Registry name | Executable used by the adapter | Role |
| --- | --- | --- |
| `codex` | `codex` | Provider adapter |
| `claude-code` | `claude` | Provider adapter; `claude` is also an adapter alias |
| `opencode` | `opencode` | Provider adapter |
| `workspace.paths` | `tactus-effect-host workspace-paths` | Observes final workspace path changes around provider calls |

Provider/effect commands are argument arrays, not shell strings. They run as
trusted local subprocesses in the project root and inherit the launching
environment, including native provider credentials.

An offline smoke probe checks executable discovery and version output. Select
only plugins that are installed; running `tactus smoke` without names selects
all configured providers and effects.

```powershell
tactus smoke codex workspace.paths
```

The command above does not send a model prompt. `tactus smoke codex --live`
does send a minimal live request and can consume provider quota.

## 6. Generate, review, check, and run

`generate` is an explicit live provider operation. Authenticate the native CLI
outside Tactus, review `.tactus/tactus.toml` and `.tactus/PROMPT.md`, and then
choose one configured provider:

```powershell
tactus generate --provider codex `
  "Create 010_inventory.hs that lists the current directory and prints a short summary."
```

Equivalent provider names are `claude-code` and `opencode`. The provider works
directly in the project and may create one or more files under
`.tactus/scripts/`. Tactus lists the result but deliberately does **not** run
generated code.

Use the review boundary explicitly:

```powershell
tactus list
Get-ChildItem .tactus\scripts
Get-Content .tactus\scripts\*.hs
tactus check
tactus run
```

In a Git worktree, also review `git status --short` and `git diff` before
execution.

Read every generated Haskell program before `run`. A script can use ordinary
Haskell `IO`, launch commands, read credentials available to the process, or
invoke another provider. Neither Clef nor Tactus is a sandbox or authorization
layer.

## 7. Which commands can contact a provider?

| Command | Model request? | Notes |
| --- | --- | --- |
| `tactus init`, `list`, `prompt`, `doctor`, `check` | No | Local workspace and toolchain operations |
| `tactus smoke NAME` | No by default | Provider executable/version probe; the selected CLI must exist |
| `tactus smoke NAME --live` | Yes for a provider | Sends a minimal documented live prompt |
| `tactus generate ...` | Yes | Selected provider may modify the workspace |
| `tactus run` | Depends on the Haskell program | `invoke`/`invokeWith` calls are live; a local-only script need not contact a provider |

CI uses fake providers and does not authenticate or make live model requests.
Provider CLI compatibility, account permissions, endpoint policy, cost, and
live behavior remain local acceptance responsibilities.

## Next reading

- [Architecture](architecture.md) explains component and trust ownership.
- [Local plugin protocol v1](reference/plugin-protocol-v1.md) defines the JSONL wire contract.
- [Support matrix](reference/support-matrix.md) separates gates from unverified live behavior.
- [Troubleshooting](troubleshooting.md) covers toolchain, encoding, and plugin failures.
- [Roadmap](roadmap.md) distinguishes active hardening from frozen or exploratory work.
