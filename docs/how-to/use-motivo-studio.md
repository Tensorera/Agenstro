# Use Motivo Studio

Motivo Studio is a Windows editor for a direct, Jupyter-style Python script.
Its React + TypeScript interface is built with Vite and hosted locally by
pywebview. Monaco provides project-file editing, syntax highlighting, and
diffs. xterm.js renders a pywinpty/ConPTY PowerShell or Bash session so you can
run Codex, Claude Code, or OpenCode without a Studio-owned agent connection.

## Prerequisites

- Windows and CPython 3.12;
- at least one supported shell: PowerShell 7, Windows PowerShell, Git Bash, or
  WSL Bash;
- an installed coding-agent CLI if you want agent assistance; and
- Git for automatic tracked-file rollback.

Windows Terminal is not required.

Start Studio from the exact project directory:

```powershell
Set-Location D:\Projects\my-project
motivo-studio
```

`tactus studio` opens the same application. Startup does not search for an
Tactus project above or below the current directory.

## Initialize or resume the project

On startup:

1. Studio loads `<cwd>/.tactus/config.json` when it exists.
2. Otherwise it creates a project-local `.tactus`.
3. If `<cwd>` is not itself a Git root, Studio asks whether to run `git init`
   in that exact directory.

Choosing **No** keeps the project runnable, but failed cells cannot restore
tracked files automatically.

Studio also creates or refreshes:

```text
.tactus/main_script.py
.tactus/agent-instructions.md
AGENTS.md
CLAUDE.md
```

Managed Tactus blocks are updated idempotently. Existing text outside the
managed block is preserved.

The generated prompt points the agent to the local Clef documentation home,
API and entity indexes, Profiles reference, and first-workflow tutorial when
that documentation tree is available. If no local tree can be found, it links
to the maintained repository documentation. The prompt asks the agent to
consult those public references before SDK implementation files. This is
behavioral guidance, not a filesystem restriction; native bypass/full-access
launch options remain under user control.

## Understand the window

- **Project** shows the exact root, Git/runtime state, and script cells.
- **Editor** uses Monaco for project files and the real
  `.tactus/main_script.py`; **Diff** compares the loaded version with the
  current buffer or an external edit.
- **Terminal** uses xterm.js for a PowerShell or Bash PTY in the project root.
  The shell selector switches between the available choices, and **Restart**
  replaces only that terminal session. **Enter** keeps its normal terminal
  meaning. In the Codex TUI, **Shift+Enter** inserts a new line, and the native
  **Ctrl+J** multiline shortcut remains available.
- **Runtime log** shows durable cell attempts, outputs, errors, and warnings.
- **Folder**, **Notebook**, and **Artifacts** open the corresponding paths.
- **Sync prompts** refreshes the agent instruction files.
- **Recover** reconciles a stale running cell after a host failure.

There is no Connect button, Studio chat session, permission dialog, approval
batch, or Goal mode in the production path.

## Start a coding agent

Click the Terminal tab and run the provider yourself:

```powershell
codex
```

or:

```powershell
claude
```

or:

```powershell
opencode
```

During trusted local debugging, Codex's native full-access launch is:

```powershell
codex --dangerously-bypass-approvals-and-sandbox
```

That exact flag is the native Codex “always approve” behavior. It is a
provider launch choice, not a Studio toggle.

The native agent owns authentication, approvals, permissions, structured
questions, model selection, tools, MCP, context management, and conversation
history. Studio displays that provider TUI in xterm.js but does not read or
mirror the provider state into a second agent sidebar. pywinpty carries the
ConPTY byte stream; xterm.js owns terminal rendering, input, selection,
scrolling, and resizing.

Regular **Enter** remains the terminal application's submit key. For a real
Windows console application, **Shift+Enter** is forwarded as a native modified
Enter key when ConPTY requests Win32 input records; this gives PowerShell and
compatible native TUIs their own multiline behavior. If that protocol is not
active, choose the portable mode in the terminal toolbar: **Codex / OpenCode**
sends Ctrl+J, while **Claude Code** sends its terminal-independent
backslash-then-Enter fallback. The portable modes are coding-agent adapters,
not a promise that every Bash/readline application treats Shift+Enter as a
newline.

Studio leaves composition keystrokes to xterm.js and serializes terminal
writes, so a committed Chinese IME string cannot be overtaken by the following
Enter. The terminal font list also includes common CJK fallbacks.

At startup the agent reads its project instruction file. If an existing agent
session predates the Tactus block, use **Sync prompts**, then ask the agent
to reread `AGENTS.md` or start a new native session.

## Edit the direct script

The agent or user edits:

```text
.tactus/main_script.py
```

Use `# %%` to split runnable cells:

```python
# %% Build
import subprocess

subprocess.run(["python", "-m", "pytest", "-q"], check=True)

# %% Summarize
from pathlib import Path

Path("result.md").write_text("# Tests passed\n", encoding="utf-8")
```

A file without markers is one cell. `# %% [markdown]` creates notes that are
not executed, but their content must still be valid Python comments or string
literals because the complete file is compiled. Each code cell gets a fresh
Jupyter kernel, so repeat imports and persist cross-cell state in files or
artifacts.

Studio polls for agent edits. If the agent changes the file while the Studio
editor has unsaved text, Studio reports an external-edit conflict instead of
overwriting either side. Reload or reconcile the content explicitly.

## Check and run

Before formal execution, select **Check** or run:

```powershell
tactus script-check --root .
```

This parses and compiles the complete file and every executable cell without
executing them.

Select **Run** or run:

```powershell
tactus script-run --root .
```

Run saves and validates the script, then sends pending cells to the core
runtime in source order. Each cell is composed and executed as a durable
attempt:

- success creates a Git checkpoint when Git is enabled;
- failure restores tracked files to the pre-cell boundary and stops the
  script;
- unchanged successful cells are skipped on the next Run; and
- editing a cell changes its digest and makes that version pending.

There is no separate approval step.

After a run, inspect the durable result in **Runtime log** or run:

```powershell
tactus status --root .
```

Use `.tactus/main-thread.ipynb` or the execution notebook referenced by the
cell attempt for complete evidence. For a task that creates or changes a
deliverable, ordinary shell commands are authoring-time diagnostics rather
than a substitute for this edit → check → run → inspect lifecycle. Purely
read-only questions and actions the user explicitly requests as direct
commands do not need a formal cell.

## Repair a failed cell

When a cell fails:

1. Read the error in **Runtime log**.
2. Open **Notebook** for complete execution evidence when needed.
3. Edit the same `# %%` cell in `main_script.py`.
4. Check the script again.
5. Run it again.

The core runtime records the new attempt as a same-phase repair of the failed
cell. Automatic agent repair is not enabled; the native agent can perform the
steps after you give it the failure prompt.

## Continue after a result

Every terminal attempt writes:

```text
.tactus/continuations/latest.md
.tactus/continuations/latest.json
.tactus/continuations/events.jsonl
```

`latest.md` contains a suggested prompt for the next agent turn. Delivery is
manual in the current version: paste it into the native coding-agent terminal.
Studio does not inject text automatically because a generic terminal cannot
safely prove that the agent is waiting at its normal input prompt.

## Record artifacts

A successful cell may print a tagged task result:

```json
{
  "schema": "clef_sdk.task-result/v1",
  "result": {"tests_passed": true},
  "artifacts": [
    {
      "operation": "create",
      "path": "result.md",
      "kind": "file",
      "description": "Validated result"
    }
  ],
  "summary": "Created and verified result.md"
}
```

Artifact operations are `create`, `update`, and `delete`; kinds are `file`
and `directory`. Tagged results update the logical Artifact Tree and are
archived under `.tactus/Artifact/tasks/`. Untagged output can still be a
valid cell result but does not change artifact history.

## Monitor and recover

The Runtime log is a bounded view over SQLite cell state. Full evidence remains
under:

```text
.tactus/state.sqlite3
.tactus/runs/
.tactus/main-thread.ipynb
.tactus/logs/studio.jsonl
.tactus/continuations/events.jsonl
```

If Studio or the host exits while a cell is running, reopen the same project
and select **Recover**. Do not use Recover to take ownership from a cell that
is still genuinely running.

Coordinator Stop controls are disabled in the current Studio. Wait for the
formal cell to finish; use provider-native Ctrl+C only for the coding-agent
terminal, not as a runtime rollback mechanism.

## Resume later

Close Studio only while no formal cell is active. Return to the same folder
and run `motivo-studio` again. The direct script, runtime attempts, Git
history, artifacts, and manual continuations remain project-local.

Read [Known limitations](../reference/known-limitations.md) before relying on
the current Studio for concurrent or autonomous operation.
