# Motivo Studio architecture

This page describes the production Studio path used during the current test
phase. Studio deliberately favors a small, runnable integration over a
provider-specific agent gateway.

## Product shape

Studio is a Windows editor around one project-local Python script. Its React +
TypeScript interface is built with Vite and hosted locally by pywebview:

```text
┌──────────────────┬─────────────────────────────────────────────┐
│ Project/runtime  │ MONACO: .tactus/main_script.py           │
│                  │ files, syntax highlighting, and diffs       │
│ Script cells     ├─────────────────────────────────────────────┤
│ and status       │ XTERM.JS TERMINAL      │ RUNTIME LOG         │
│                  │ PowerShell or Bash PTY │ cell evidence       │
└──────────────────┴─────────────────────────────────────────────┘
```

The user starts `codex`, `claude`, or `opencode` manually in the xterm.js
PowerShell or Bash terminal. A small pywebview JavaScript API delegates project
and runtime operations to the existing `DirectStudioController`; changing the
window implementation does not change its direct-script, Jupyter, SQLite, or
Git semantics. The native coding-agent process owns its conversation,
authentication, model, permissions, approvals, tools, and interactive
questions. Studio does not create or resume an agent session and does not
translate the provider protocol.

The coding agent and Studio edit the same file:

```text
<project>/.tactus/main_script.py
```

Studio polls the file and projects external edits into the editor. A
compare-and-swap digest prevents a stale Studio buffer from silently
overwriting a newer agent edit.

## Direct script

`main_script.py` is ordinary Python. A file without markers is one runnable
cell. `# %%` comments split it into Jupyter-style cells:

```python
# %% Prepare inputs
from pathlib import Path

Path("input.txt").write_text("ready\n", encoding="utf-8")

# %% Validate
assert Path("input.txt").read_text(encoding="utf-8") == "ready\n"
```

`# %% [markdown]` creates a non-executable notes cell. Because the complete
file must still compile as Python, write those notes as comments or string
literals. Each executable cell must be self-contained because the runtime
starts a fresh Jupyter kernel for every formal attempt. State that must cross
cells belongs in project files or artifacts, not in Python memory.

The direct file is authoritative. The former
`.tactus/main-script.md`, append-only composition journal, approved
batches, and frozen revisions are not part of the production Studio path.
When an older project is opened, Studio can render its active legacy cells
into `main_script.py`; the older files remain only as migration evidence.

## Validation and execution

Studio exposes **Check** and **Run**:

```text
main_script.py
      │
      ├── Check ──> parse + compile the whole file and each # %% cell
      │
      └── Run
            ├── save with digest conflict detection
            ├── validate syntax
            └── for each pending direct cell
                  ├── Tactus.compose(source)
                  ├── Tactus.run_latest()
                  ├── Git checkpoint on success
                  └── Git rollback on failure
```

There is no approval journal between the file and the runtime. The script
cell's ordinal and source digest are recorded in the durable runtime attempt,
so an unchanged successful cell is not rerun. Editing its source changes the
digest and makes that version pending.

Execution stops at the first failed cell. The failed attempt remains durable,
tracked project changes are rolled back to the pre-cell Git boundary, and the
next formal composition becomes a same-phase repair attempt. The user or
coding agent edits the failed `# %%` cell and runs the script again.

The equivalent terminal commands are:

```powershell
tactus script-check --root .
tactus script-run --root .
```

## Prompt-only agent integration

Studio installs an idempotent managed instruction block without replacing
existing user text:

| Path | Purpose |
| --- | --- |
| `.tactus/agent-instructions.md` | Canonical Tactus workflow prompt |
| `AGENTS.md` | Codex/OpenCode-compatible project instructions |
| `CLAUDE.md` | Claude Code entry point that imports `AGENTS.md` |

The **Sync prompts** button and `tactus prompt-sync --root .` refresh these
files. The prompt tells an agent how to edit, check, run, inspect, and repair
`main_script.py`.

This is instruction following, not a capability boundary. Studio does not
inject an epoch token, require a context acknowledgement, prevent direct
project edits, or inspect whether a model retained the instructions.

## Web terminal boundary

The terminal bridge exposes only two shell identifiers: `powershell` and
`bash`. PowerShell prefers `pwsh` and falls back to Windows PowerShell. Bash
prefers a Git Bash executable outside System32; when that is unavailable it
uses `wsl.exe --cd <project> --exec bash`. An unavailable shell is reported
before Studio attempts to start it.

xterm.js owns the terminal grid, keyboard input, selection, scrolling, and
resize behavior. pywinpty 3.x creates the ConPTY process in the fixed project
root. A reader thread appends bounded sequence-numbered output chunks;
`terminal_read` polls those chunks without blocking, while separate bridge
calls write input, resize, and close the PTY. Windows Terminal is not a runtime
requirement. The user launches the preferred coding agent exactly as they
would in an ordinary shell:

```powershell
codex
# or
claude
# or
opencode
```

Provider-native UI handles login, permission requests, structured questions,
tool output, session management, and provider slash commands. Studio neither
adds `--dangerously-bypass-approvals-and-sandbox` nor creates a second
approval layer. Users who want such a provider option supply it themselves
when starting the agent.

For trusted local Codex debugging, that explicit launch is:

```powershell
codex --dangerously-bypass-approvals-and-sandbox
```

Closing, restarting, or switching the terminal affects only the PTY shell and
coding-agent process. It does not remove `main_script.py`, SQLite cell history,
Git boundaries, artifacts, or execution notebooks. Closing the WebView closes
all PTY sessions and then closes the controller.

## Runtime and filesystem boundary

Studio uses exactly the directory from which it starts. It reads or creates
only `<cwd>/.tactus`; it never searches parents or descendants and never
places runtime control files beside the project. Git leases, kernel readiness
rendezvous, and other disposable runtime data also stay below
`<cwd>/.tactus`. The agent, compiler, debugger, tests, and Jupyter cells all
operate in that same project.

With Git enabled, a formal cell has one filesystem mechanism:

1. establish a pre-cell commit boundary;
2. execute in the project root;
3. commit an allowlisted successful result, or restore tracked files to the
   boundary on failure.

Without Git, execution remains available but rollback is unavailable. The
runtime does not promise ownership attribution when another process edits the
project during a cell; the current production assumption is one writer during
formal execution.

## Continuations

After every formal cell result, Studio writes a durable continuation record:

```text
.tactus/continuations/
├── events.jsonl
├── latest.json
└── latest.md
```

The record contains the script cell, engine attempt, result, error, warnings,
and a suggested follow-up prompt. Delivery is currently
`pending_manual`: the user copies or sends the prompt to the coding agent.

Automatic terminal injection is intentionally deferred. Studio does not own
the provider session and cannot reliably distinguish an agent input prompt
from a permission prompt, a shell prompt, or an active turn. Future automation
can consume the same durable outbox after it has a provider-safe readiness and
idempotency protocol.

## Features deliberately out of the production path

The current Studio does not expose:

- automatic Codex/Claude/OpenCode connection or session resume;
- provider event or permission projection;
- Discussion versus Compose session modes;
- append-only composition journal or approval batches;
- RunCoordinator stage interruption;
- synchronous automatic repair; or
- Goal mode.

The lower-level runtime and legacy CLI may still contain compatibility
surfaces for some of these concepts. They do not define the production Studio
workflow.

See [Known limitations](../reference/known-limitations.md) for the explicit
current boundary.
