# Motivo Studio 0.3

Motivo Studio owns the task method and desktop interface for a Tactus workspace.
It is a TypeScript, React, Vite, and Electron application. Tasks use bounded
agent calls and local report history; Tactus owns workflow discovery, plugin
configuration, health checks, process supervision, and diagnostic run history.

The canonical user guide is [Motivo Studio](../docs/motivo-studio.md), with
installation prerequisites maintained in [Install Agenstro](../docs/install.md).
This component README stays focused on application development and packaging.

The task method offers investigate, try, integrate, and conclude as optional
actions. It does not require a Haskell workflow for ordinary work. Every agent
call still passes through Tactus. Existing views consume the versioned
`tactus.control/v1` API and launch normal Tactus CLI actions. There is no source
editor, general terminal, background scheduler, or direct Tactus journal reader.

Motivo Studio is licensed with the rest of Agenstro under
[GNU AGPL v3.0 only](../LICENSE).

## Install on Windows x64

Requirements:

- Node.js 22.12 or newer
- the Rust `tactus` executable on `PATH`
- an installed Haskell toolchain for `Check` and `Run`

From the Agenstro repository root, install the locked dependencies and the
current Windows x64 desktop build:

```powershell
npm --prefix motivo-studio ci
npm --prefix motivo-studio run install:windows
```

This replaces `%LOCALAPPDATA%\Programs\MotivoStudio` and adds that directory,
which contains `motivo-studio.exe`, to the user `PATH`. It is a repository-owned
Windows installation, not an npm global package. Open a new terminal to receive
the `PATH` change, then start Studio with no argument or pass one initialized
Tactus workspace:

```powershell
motivo-studio
motivo-studio 'D:\work\Project with spaces'
```

The command shape is `motivo-studio [WORKSPACE]`. Relative paths are resolved
from the calling terminal. `--workspace PATH` is also accepted, and `--` can
precede a path whose first character is `-`; do not pass more than one workspace
or combine the positional and `--workspace` forms.

Motivo is single-instance. A later invocation without a workspace focuses the
existing window. With a workspace, Tactus validates and switches that window
before Motivo focuses it; a failed validation keeps the current workspace.

To upgrade after updating the checkout, close Studio and repeat the install
command. To remove the recognized installation and its user-`PATH` entry, close
Studio and run from the repository root:

```powershell
npm --prefix motivo-studio run uninstall:windows
```

## Develop from the checkout

From this directory, Electron Forge development remains:

```powershell
npm install
npm start
```

For a development build of Tactus that is not on `PATH`, set an absolute
executable path in the main-process environment before starting Studio:

```powershell
$env:MOTIVO_TACTUS_BIN = 'D:\path\to\tactus.exe'
npm start
```

Use **Open workspace** for a folder that already contains `.tactus`. Use
**Initialize folder** to run `tactus init` in the selected folder and then open
its control snapshot. Initialization must be able to discover Clef through the
normal Tactus rules, such as `TACTUS_CLEF_SDK`.

Choose the workspace root itself, not a nested folder. Motivo requests exact
root validation so the main process's private redaction root is identical to
the workspace Tactus opened.

## Views

- **Tasks**, the default, accepts a goal, constraints, and provider, then runs
  a chosen provider-call budget. It shows report history, call timing,
  findings, uncertainty, decisions, artifacts, reported checks, and user notes.
- **Overview** shows workspace health, script and plugin totals, and recent run
  outcomes.
- **Workflow** shows Tactus-ordered Haskell scripts and starts Generate, Check,
  or Run with explicit script selection. Generate accepts an optional registered provider.
- **Plugins** shows provider, effect, and generic-plugin projections and starts
  offline or live smoke probes.
- **Runs** pages through the typed event projection for one opaque run ID. Open
  future event kinds remain available under collapsed technical details.
- **Sessions** retains compatibility with durable Tactus decision sessions, explains findings and
  stakes, compares options on shared coordinates, and returns one turn-scoped
  answer with an optional note. A stale or already-consumed turn is refetched
  instead of retried.

Workflow actions and task execution are serialized. The existing action drawer's visible
log contains only `[state]`, `[info]`, `[warning]`, and `[error]` plus bounded
natural-language messages. Raw stdout/stderr, exit information, event kinds,
and structured payloads are bounded and available under collapsed technical
details; stderr is not classified as an error by itself. If the raw action
projection reaches its byte or frame budget, Studio continues draining the
Tactus child, discards later raw projection, and emits exactly one canonical
`[warning]`. Projection loss never kills Tactus or changes the action's exit
status. Cancel asks the main process to terminate that Tactus process. When the
action finishes, the renderer refreshes the Studio snapshot.

Studio starts at 120% zoom for more readable desktop text. Hold `Ctrl` while
using the mouse wheel, or use `Ctrl` + `+` / `-`, to zoom between 80% and 200%.
Use `Ctrl` + `0` to return to 120%.

Cancellation uses Electron/Node's platform process termination and is
best-effort. Tactus supervises the subprocesses it starts, but forcibly killing
the Tactus leader is not a universal process-tree guarantee on every operating
system. Do not treat Studio cancellation as a security boundary for untrusted
plugins.

## Authority boundary

```text
React renderer
    | strict Zod IPC; no path, Node, Electron, network, or filesystem authority
    v
Electron preload (context isolation + sandbox)
    |
    v
Electron main -- owns selected root, task method, task records, and child handles
    | argv array, shell: false, bounded stdout/stderr
    v
tactus dispatch --namespace provider
       + studio inspect / studio events / session list|show|answer
       + init / generate / check / run / smoke
```

The renderer receives a random workspace handle and a redacted
`agenstro.studio/v1` snapshot. Workspace control calls use that handle rather
than passing the authoritative absolute root.
The main process does not read Tactus config, scripts, journals, or session
files directly. It reads `.motivo/METHOD.md` and owns atomic
`.motivo/tasks/<uuid>.json` records. Control JSON and task reports are bounded
and validated before they cross IPC. Task reports contain business content;
the redaction contract of the Studio snapshot does not apply to them. Existing
action output and open trace payload strings are scrubbed for the selected root.

The compatibility session interface is intentionally narrower than Tasks. It does not
create briefs, invoke a planner, apply defaults on a timer, or expose a dead
`advance` action; Tactus must publish a later brief through a future, explicit
planner contract. Every session IPC call is bound to the opaque workspace
handle that produced its view, so a delayed answer cannot be redirected into a
newly opened workspace.

## Task method implementation

`src/main/tasks/method.ts` holds the default method and report instructions.
An optional project `.motivo/METHOD.md` replaces method text only; the
`motivo.task/v1` record and structured-report contract stay fixed. The service
and store are separate from Tactus sessions and Segno persistence.

The default continuation budget is four provider calls, with a maximum of
twenty. Calls include the lead and up to three concurrent investigation
branches; a lead integration call is reserved before branches start.
Investigators are asked not to edit, but share the working environment.
Parallel writing and read-only sandbox enforcement are not supplied.

Pause takes effect after current calls finish. Budget exhaustion saves a
handoff. An interrupted process or invalid post-execution report stops without
automatic retry; continuing an unknown outcome requires a reconciliation note.
Recent reports provide bounded context for a new call rather than resuming a
native agent session. `completed` is an agent's delivery claim, and report
validation does not verify project correctness.

The lead may use project tests and configured plugins, or create a focused
plugin and fixtures when the task needs a missing observation. No domain
validator or mandatory workflow topology is built into Motivo. See
[ADR-0007](../docs/adr/0007-motivo-task-method.md) for the ownership decision.

Trace-v1 events may include Tactus's additive `presentation` field with one
closed category and a bounded natural-language message. Studio displays that
canonical projection and does not infer user-facing severity from open event
kinds or legacy payloads. The trace remains redacted diagnostic evidence, not
a replay contract or a source of workflow state.

Electron uses context isolation, renderer sandboxing, no Node integration, a
locked-down `motivo://app` asset protocol, denied navigation/window creation,
and denied permissions. Tactus is external by design and is not bundled into the
desktop package.

## Development checks

```powershell
npm run typecheck
npm run lint
npm test
npm run format:check
```

The unit suite uses fake IPC, Tactus processes, and task reports. It covers
budget accounting, method overrides, report handling, task persistence, and
interruption without calling a real model provider. Electron packaging is available through `npm run package`
or `npm run make`.
