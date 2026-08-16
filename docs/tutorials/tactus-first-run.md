# Run your first Tactus project

This tutorial opens an ordinary project, starts a coding agent in Studio's
xterm.js PowerShell or Bash terminal, and runs a direct Python cell with Git
rollback.

## 1. Install Tactus

From the Agentro repository root:

```powershell
py -3.12 -m venv tactus-runtime\.venv
.\tactus-runtime\.venv\Scripts\python.exe -m pip install `
  -c .\tactus-runtime\constraints-windows-py312.txt `
  -e .\tactus-runtime
```

Activate the environment or put its Scripts directory on `PATH`.

## 2. Open the exact project

```powershell
New-Item -ItemType Directory -Force D:\Projects\tactus-demo
Set-Location D:\Projects\tactus-demo
motivo-studio
```

Studio uses exactly `D:\Projects\tactus-demo`. It creates or resumes only
that directory's `.tactus` and never adopts one from a parent or child.

## 3. Enable Git rollback

If the folder is not already an independent Git root, Studio asks whether to
initialize Git. Choose **Yes** for this tutorial.

Tactus does not rewrite Git user configuration. Before each formal cell it
establishes a baseline. Success creates a checkpoint; failure restores tracked
files to the baseline and preserves ambiguous untracked paths with warnings.

Do not edit tracked files from another process while the cell runs.

## 4. Start the coding agent

Open the **TERMINAL** tab and start an installed, already authenticated agent:

```powershell
codex
```

You can use `claude` or `opencode` instead. The provider's native terminal UI
owns login, permissions, questions, and conversation history. There is no
Studio Connect button. xterm.js renders a pywinpty/ConPTY session rooted in the
project. PowerShell is selected by default when available; use the shell
selector for Git Bash or WSL Bash. Windows Terminal is not required.

Use **Shift+Enter** for a multiline prompt. Studio preserves native Windows
modified-key events when the child requests them. Otherwise leave the toolbar
on **Codex / OpenCode**, or select **Claude Code** for its portable
backslash-then-Enter fallback. Regular **Enter** still submits.

Studio has already installed a Tactus block in `AGENTS.md` and
`CLAUDE.md`. Ask the agent:

> Edit `.tactus/main_script.py` so one cell creates `result.md`, verifies
> its content, and prints a tagged task-result JSON object.

The agent edits the same Python file shown in Monaco. If it changes the file
while your buffer is dirty, Studio opens the Monaco diff instead of silently
overwriting either version.

## 5. Review the direct cells

The script can look like:

```python
# %% Create and verify result
import json
from pathlib import Path

result = Path("result.md")
result.write_text("# Tactus demo\n", encoding="utf-8")
assert result.read_text(encoding="utf-8") == "# Tactus demo\n"

print(
    json.dumps(
        {
            "schema": "clef_sdk.task-result/v1",
            "result": {"verified": True},
            "artifacts": [
                {
                    "operation": "create",
                    "path": "result.md",
                    "kind": "file",
                    "description": "Verified tutorial output",
                }
            ],
            "summary": "Created and verified result.md",
        }
    )
)
```

`# %%` uses the same lightweight cell convention as VS Code Python files. A
file without markers is one cell.

## 6. Check and run

Select **Check**. The script is compiled without being executed.

Select **Run**. Studio saves and validates the file, then submits its pending
cells to the core runtime. The equivalent terminal commands are:

```powershell
tactus script-check --root .
tactus script-run --root .
```

The Runtime log shows the engine cell ID, status, attempt number, output
preview, and any warnings.

## 7. Inspect artifacts and continuations

After success:

- `result.md` exists in the real project;
- `.tactus/Artifact/tasks/` contains the accepted result envelope;
- `.tactus/artifact-tree.json` contains the reported artifact mutation;
- `.tactus/Artifact/workspace/result.md` normally hardlinks the real file;
- `.tactus/continuations/latest.md` suggests the next agent prompt.

Continuation delivery is manual. Paste `latest.md` into the agent if you want
it to plan another cell.

## 8. Try a repair

Change the cell so it writes a temporary value and then raises:

```python
result.write_text("broken\n", encoding="utf-8")
raise RuntimeError("intentional tutorial failure")
```

Run again. The runtime records a failed attempt and restores the tracked
project state. Read the error, repair the same `# %%` cell, check it, and run
again. The new engine cell is recorded as the next attempt in the same phase.

Git cannot safely delete ambiguous untracked files, so a newly created
non-allowlisted file may remain with a residue warning.

## 9. Resume later

Close Studio only after the formal cell finishes. Later:

```powershell
Set-Location D:\Projects\tactus-demo
motivo-studio
```

Studio reloads the direct script and durable runtime history from this exact
folder. Starting from a nested folder creates or resumes a distinct Tactus
project.

Next, read [Use Motivo Studio](../how-to/use-motivo-studio.md) and
[Known limitations](../reference/known-limitations.md).
