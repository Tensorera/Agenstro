---
title: Motivo Studio
status: alpha
owners: [motivo]
last_verified: 2026-09-05
applies_to: "Motivo Studio 0.3.0 and Tactus control APIs"
platforms: [windows, ubuntu]
---

# Motivo Studio

Motivo Studio `0.3.0` is the task method and desktop interface for an installed
Rust Tactus runtime. Give it a concrete goal and constraints; it can investigate,
attempt changes, integrate findings, and report a result through bounded agent
calls. Reusable Haskell workflows, configured plugins, run evidence, and existing
decision sessions remain available in the workspace views.

## Install and start on Windows x64

Use [Installation](install.md) for prerequisites and the canonical deployment
procedure. From the repository root, the Windows x64 application installation
itself is:

```powershell
npm --prefix motivo-studio ci
npm --prefix motivo-studio run install:windows
```

The installer replaces `%LOCALAPPDATA%\Programs\MotivoStudio` and adds that
directory, which contains `motivo-studio.exe`, to the user `PATH`. It does not
publish or globally install an npm package. Open a new terminal to receive the
`PATH` change, then start the UI or open one initialized workspace:

```powershell
motivo-studio
motivo-studio 'D:\work\Project with spaces'
```

The command shape is `motivo-studio [WORKSPACE]`. It resolves a relative path
from the calling terminal. The equivalent explicit form is `--workspace PATH`;
use `--` before a path whose first character is `-`. Supply exactly one
workspace and do not combine the positional and explicit forms.

Motivo is single-instance. A second invocation without a workspace focuses the
existing window. With a workspace, Tactus validates and switches the existing
window before Motivo focuses it; invalid roots leave the current workspace open,
show an error, and focus that same window.

Close Studio and run `npm --prefix motivo-studio run install:windows` again
after updating the checkout to upgrade. Close Studio and uninstall the
recognized per-user application and its user-`PATH` entry with:

```powershell
npm --prefix motivo-studio run uninstall:windows
```

Development remains separate from the installed launcher:

```powershell
npm --prefix motivo-studio start
```

By default Electron resolves `tactus` from `PATH`. A development or installed
application may point at an exact binary by setting `MOTIVO_TACTUS_BIN` before
starting Studio.

Use **Open workspace** for a project that already contains `.tactus`, or
**Initialize folder** to run `tactus init` and then open it. Initialization
still needs a discoverable Clef SDK; when automatic checkout discovery is not
available, initialize from the CLI with `tactus init --sdk <clef-sdk>` first.

Select the workspace root itself, not a descendant. Studio inspection uses an
exact-root guard so its private redaction authority cannot differ from the
workspace Tactus discovered.

## Views and actions

**Tasks** is the default entry point. Create a task with its goal, constraints,
and a configured provider. Choose the provider-call budget for the next stretch
of work, then continue. Task reports show findings, uncertainty, decisions,
artifacts, reported checks, next actions, and call timing. Add a note to answer
a question or steer later work.

The other views retain explicit workspace operations:

- **Overview** summarizes Tactus doctor checks, workflow entries, registries,
  recent runs, and the active action.
- **Workflow** displays numbered runnable entries in Tactus order and separates
  helper modules. It can request Generate, Check, or Run with explicit script
  selection; opening a task does not generate a workflow automatically.
- **Plugins** groups providers, effects, and open generic plugins. It shows only
  redacted metadata and can start offline Smoke. Live Smoke is a separate,
  explicit action because it may contact or bill a provider.
- **Runs** pages through open `agenstro.trace/v1` events and shows the terminal
  summary when one exists. Canonical Tactus presentation messages form the
  visible log; unknown and legacy event data remains available in collapsed
  technical details.
- **Sessions** lists bounded `agenstro.session/v1` views. It shows findings
  before one pending question, aligns option coordinates for comparison,
  keeps consequences beside their options, distinguishes sourced findings
  from inference, and shows both the necessary and conditional decision
  roadmap. A person can return one option and an optional bounded note. This
  remains a compatibility view for existing Tactus sessions, separate from
  Motivo task questions and reports.

## Task method, budgets, and continuation

The default method offers four actions:

| Action | Purpose |
| --- | --- |
| Investigate | Resolve a concrete uncertainty through relevant sources or a small experiment |
| Try | Make a coherent change or test an approach |
| Integrate | Use returned evidence to revise a decision or combine results |
| Conclude | Deliver the work and describe the remaining limits |

These actions are choices, not a required sequence. Simple work may finish in
one call. The agent can use existing tests and plugins. If a missing observation
tool directly blocks the task, the lead may create a small project-local
plugin with fixtures and invoke it through Tactus. Neither Motivo nor Tactus
imposes a universal definition of project correctness.

Each continuation permits four provider calls by default; choose between one
and twenty. The budget includes the lead and any investigator calls. One call
is a native coding-agent episode, so it may contain multiple model/tool steps.
This budget is not a token, cost, or wall-clock ceiling; Tactus retains its own
configured execution deadlines.

When useful, the lead may ask up to three independent investigation questions.
Motivo starts only branches that fit the remaining budget while reserving a
lead call to integrate their findings. Investigators receive separate contexts
and are asked to avoid edits. Their environment is shared: this instruction is
not an enforced read-only sandbox or isolation for parallel writes. Dependent
edits stay with the sequential lead.

**Pause** lets the current action and active investigations finish, then saves
the handoff. Exhausting the budget also pauses. Continuing starts fresh agent
calls with a bounded recent report history and source references; it does not
resume an in-memory agent conversation or replay previous tools.

| Task state | Meaning |
| --- | --- |
| `ready` | Created, with no active execution |
| `running` | Motivo is spending the current call budget |
| `paused` | The handoff is saved and another continuation can be requested |
| `needs_input` | The agent has asked a question; supply an answer in a note |
| `completed` | The agent reports that it delivered the requested work |
| `failed` | A known invocation failure stopped this stretch of work |
| `outcome_unknown` | Execution may have changed external state without a usable committed result |

An agent's `completed` report and listed checks are claims and observations to
review, not independent verification. Motivo validates report structure. It
does not certify project correctness or prove that its method improves a model.

Closing the application can interrupt an active call. Interrupted work and
invalid post-execution reports are not automatically retried. Inspect the
workspace and relevant external effects, then supply a reconciliation note
before continuing `outcome_unknown`. A note records your decision; it cannot
undo or establish exactly-once external work.

## Method customization and local records

Create `.motivo/METHOD.md` to replace the default method instructions for this
project. Without that file, Motivo uses its built-in method. The override
changes guidance, not the required structured-report protocol or Tactus
execution boundary. It should describe useful ways of deciding what to do next,
rather than prescribe extra stages for every task.

Motivo owns atomic task records at `.motivo/tasks/<uuid>.json`. They contain
goals, constraints, user notes, timing, and reports, including business content.
When a final response cannot be decoded as a report, the failed call retains
up to 8,000 characters of raw text for diagnosis, with a truncation marker.
The UI shows it as plain text in a collapsed panel; subsequent prompts do not include it.
They are separate from Tactus's redacted run evidence and from
`.tactus/sessions` and Segno state. Preserve or remove these local task records
according to the project's data policy; do not treat them as workspace backups.

## Existing Tactus action drawer

The action drawer displays only `[state]`, `[info]`, `[warning]`, and `[error]`
plus bounded natural-language messages while Electron owns the top-level
Tactus child. It strictly recognizes those exact tags from Tactus human output;
untagged stdout/stderr stays inside a collapsed raw-output panel because stderr
alone does not imply an error. If that raw projection reaches its byte or frame
budget, Electron continues draining both pipes, omits subsequent raw frames,
and emits one `[warning]`; it does not kill Tactus or replace the child's real
terminal status. Workflow actions are serialized with task execution. Closing the window or
pressing Cancel asks that process to stop; a terminal action event reports the
observed result. Cancellation is best effort at the desktop boundary, while
Tactus remains responsible for supervised plugin descendants.

The window opens at 120% zoom. `Ctrl` + mouse wheel and `Ctrl` + `+` / `-`
adjust the complete interface between 80% and 200%; `Ctrl` + `0` restores the
120% default.

## Typed boundary

The renderer is sandboxed, has no Node integration, and cannot access host paths
or arbitrary Electron IPC. A context-isolated preload exposes one named,
Zod-validated method for each supported operation. Electron main retains the
workspace root and starts Tactus with an argv array and `shell: false`.

Motivo does not read `tactus.toml`, `runtime.json`, `.tactus/runs`, or
`.tactus/sessions` directly. Its main process may read the method override and
read/write its own `.motivo/tasks` records. Task provider calls use the existing
`tactus dispatch --namespace provider` boundary; Motivo does not launch native
provider executables itself.
The Rust runtime owns sorting, registry resolution, trace validation, and
resource limits through:

```powershell
tactus studio inspect --root D:\work\project --run-limit 50
tactus studio events run-... --root D:\work\project --after 0 --limit 250 --max-bytes 4194304
tactus session list --root D:\work\project --limit 50
tactus session show --root D:\work\project --session session-...
tactus session answer --root D:\work\project --session session-... --turn 3 --axis design.axis --option choice
```

Studio queries emit a `tactus.control/v1` envelope containing an
`agenstro.studio/v1` projection; session queries carry `agenstro.session/v1`.
The Studio projection omits configured command arrays,
plugin options, generation prompt text, and absolute script paths. Timestamps,
event sequences, and counts are decimal strings, avoiding JavaScript's 53-bit
integer limit. Event pages report `ok`, `partial`, or `corrupt` integrity and
refuse unsafe trace paths. An event may carry Tactus's canonical
`presentation` object with one public category and a bounded natural-language
message; its structured `data` remains redacted technical evidence. Trace pages
are diagnostic observations, not replay input or authoritative workflow state.
See the
[Studio control API v1 reference](reference/studio-control-v1.md) for the exact
envelopes, pagination rules, and limits.

Session requests also carry the opaque handle of the workspace view that
created them; Electron main rejects the request if the user opened another
workspace before it could start. Answers use the displayed turn as a second,
domain-level compare-and-set token. If another client has already consumed or
moved the turn forward, Motivo refetches the current brief instead of applying
or retrying the old choice. The workspace remains the source of truth: the
renderer never writes a session file, constructs a brief, chooses a default,
or applies a default on a timer. The existing session planner and
`session advance` remain unimplemented; Tasks use a separate method and store.
See the
[session control reference](reference/session-control-v1.md) and
[ADR-0006](adr/0006-motivo-session-pattern.md).

## Intentional non-goals

Studio does not provide:

- a background task daemon, scheduler, replay engine, artifact backup, or rollback;
- a general terminal or shell;
- a source editor or a second script-discovery implementation;
- legacy session planning, unattended session defaulting, or session
  expiry;
- provider login, credential storage, permission policy, or authentication;
- an automatic proof that an agent report or project harness is correct;
- a guarantee that cancelling a top-level command reverses external work.

Workflows and plugins remain trusted local code with the capabilities described
in the [support matrix](reference/support-matrix.md).

## Verify locally

The default tests use fake Tactus processes and task reports, and never contact
a model:

```powershell
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio run typecheck
npm --prefix motivo-studio test
npm --prefix motivo-studio run package
```

Packaging creates an unsigned current-platform application. The package does
not bundle a Tactus executable; the target machine must install Tactus or set
`MOTIVO_TACTUS_BIN`.
