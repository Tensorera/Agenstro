---
title: Motivo methods and offline reports
status: alpha
owners: [motivo]
last_verified: 2026-09-05
applies_to: "Motivo, Clef and Tactus 0.3"
platforms: [windows, ubuntu]
---

# Motivo methods and offline reports

The user's first interface is their existing coding agent. Initialize a folder,
start Claude, Codex or another supported agent there, and ask it to use Motivo.
The agent chooses a method, supplies the question and evidence, and continues
work after reading its result. Motivo Studio is the resulting offline HTML
observation page; it does not start or control agents.

## Start from your coding agent

```sh
tactus init --sdk /path/to/Agenstro/clef-sdk
```

Open your coding agent in the initialized project and ask, for example:

> Use Motivo to investigate why importing a large file stalls. Keep the work
> focused and show me the evidence before deciding what to change.

Initialization installs canonical instructions in `.tactus/skills/motivo` and
project-local discovery pointers for supported agents. If a host does not expose
skills, ask it to read `.tactus/skills/motivo/SKILL.md` directly. Existing user
instructions and skills are preserved.

The agent can point you at `.tactus/motivo/index.html`. Open that file in your
browser. Keep task instructions, model choices and feedback in the original
agent conversation. Opening the report is optional and closing it does not stop
work.

## Eight independent methods

| Method | Question it helps answer | Main material |
| --- | --- | --- |
| clarify | What outcome and constraints are actually intended? | Goal, assumptions, unknowns |
| investigate | What evidence resolves this specific uncertainty? | Findings and source locations |
| analyze | Which explanation fits the available evidence? | Hypotheses, counterevidence, decision |
| research | What external knowledge applies to this decision? | Sources, comparison, limits |
| probe | What small experiment can distinguish the hypotheses? | Prediction, actual observation, logs |
| organize | Which coherent action should happen next? | Work units and real dependencies |
| retrospect | Why did the work change direction, and what should change next time? | Expectations versus observations and decision history |
| handoff | What does the next agent need to continue without repeating work? | Current state, checks, open issues, next step |

These are choices, not an eight-stage pipeline. Every method includes a small
local reflection; it does not invoke an additional reviewer. Simple reasoning
can remain in the main agent. Stable templates consume new input rather than
requiring new Haskell scaffolding for every question.

## Script and data boundaries

```text
.tactus/scripts/        business Haskell entries
.tactus/motivoscript/   method Haskell entries and shared Motivo modules
.tactus/motivotest/     bounded experiment files, grouped by run/sample
.tactus/motivo/         method records, artifacts, index.html
.tactus/runs/           Tactus diagnostic records (unchanged)
```

Tactus `list`, `check`, and `run` accept `--scripts-dir .tactus/motivoscript`.
Omitting it keeps the existing business-script selection. In particular,
`tactus run --all` does not execute installed method templates.

For an agent preparing a retrospective:

```sh
tactus check --scripts-dir .tactus/motivoscript \
  .tactus/motivoscript/070_retrospect.hs

tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/070_retrospect.hs \
  -- --input retrospective-notes.md --agent "current coding agent"
```

The input is ordinary Markdown. The method reference suggests headings and
reflection questions; do not fabricate evidence to fill a section. The default
mode records the main agent's supplied material without contacting a provider.
Where an independent investigation is useful, select an existing Tactus provider
explicitly using the method's `--provider` option. This starts a fresh provider
invocation, not a continuation of the main CLI session. Use the installed
method's `--help` and skill references for the exact options.

A script is atomic as a work unit, not a database transaction. Failure does not
undo its earlier external effects. Parallel calls do not provide isolated
write access to a shared repository.

## Minimal experiments

Install the separate `motivo-test` executable as described in [Installation](install.md).
New workspaces register it as the `motivo.test` effect. Existing `tactus.toml`
files are preserved; add the following entry only if it is absent:

```toml
[effects."motivo.test"]
command = ["motivo-test"]
```

The probe template calls this effect through Clef and Tactus. Its Linux backend
uses Bubblewrap to expose `/project` as the read-only workspace and `/work` as
the sample directory. Experiment programs run in `/work`. Prepare fixtures in
`.tactus/motivotest/<run-id>/<sample-id>`; use relative paths for outputs and
`/project/...` for project inputs. The environment does not inherit the parent
agent's credentials or control sockets.

The default and maximum experiment timeout are 600 seconds; an explicit value
must be an integer from 1 through 600. Preparation, payload compilation, and
execution share one monotonic deadline. The experiment namespace is terminated
at the deadline; saving the final observation is not extra experiment time.
Framework installation and Haskell template preparation are separate from this
bounded experiment and must not be presented as experiment execution.

The current strict backend is Linux only and requires usable Bubblewrap user
namespaces. Unsupported or unavailable isolation refuses execution. A working
directory or a prompt saying “do not edit” is not an equivalent fallback.
Other methods and offline reports remain available without this backend.

A nonzero exit, timeout, or missing terminal result is evidence to inspect.
Motivo does not retry an experiment automatically or promote experimental files
into business source. The main agent chooses any subsequent business Haskell
step using the Tactus skill.

## Records and model identity

Each method writes `.tactus/motivo/runs/<run-id>`. Its request, report, metadata,
samples and artifacts are local business content. Runtime-generated times and
execution results are distinct from agent-authored judgments. Parent references
express known dependencies; timestamps alone do not establish causality.

The report records the caller's agent/model labels and any explicit provider
selection. A caller-supplied or requested model is not proof of the model that
actually ran. Missing identity remains “not reported”; Motivo does not guess it
from a CLI name or read credentials to infer an account.

Retrospectives organize expected and observed behavior, supporting evidence,
changes in judgment and the next useful action. Recorded/completed method state
does not mean the user's engineering goal has been independently verified.

## Reading a report

Reports embed their core content and styles. They need no port, server, CDN,
login, browser directory permission, or adjacent JSON fetch. Evidence can be
expanded, and the browser can reload the current file after a new snapshot is
published. Core content is readable with JavaScript disabled.

The page is a snapshot. Its generation time and last observation time describe
what has been recorded; it is not a live connection or proof that an old
in-progress call is still running. The main agent remains the place to request
status and provide direction. Large raw attachments may remain separate files;
the page identifies those rather than claiming they are embedded.

## Migration from the desktop client

The Electron task service, provider UI, execution controls and desktop installers
have been removed. Existing `.motivo/tasks` records are left untouched as
historical material. They are not converted into new samples or automatically
resumed. Tactus's existing control/session APIs and Segno are unchanged.

See [ADR-0008](adr/0008-agent-led-motivo.md) for the ownership decision and
[Observability](observability.md) for the distinction between method reports
and runtime evidence.
