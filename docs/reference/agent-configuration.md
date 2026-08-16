# Agent configuration ownership

The production Studio hosts a React + TypeScript/Vite interface in pywebview.
xterm.js renders a pywinpty/ConPTY PowerShell or Bash session rooted in the
project, and the user launches a native coding agent manually. Studio does not
require Windows Terminal, own an agent protocol, or duplicate agent
configuration.

## Ownership matrix

| Data or behavior | Owner | Studio behavior |
| --- | --- | --- |
| Project root, kernel, cell timeout, and runtime settings | `.tactus/config.json` | Loads or initializes the exact startup directory |
| User Tactus defaults | `~/.tactus/settings.json` | Project settings recursively override defaults |
| Main script | `.tactus/main_script.py` | Edits and projects the same file used by the agent |
| Cell attempts, errors, warnings, and outputs | Tactus SQLite and execution notebooks | Displays runtime evidence |
| Git checkpoint and rollback | Tactus runtime | Applies at formal cell boundaries |
| Agent executable and version | User and `PATH` | User launches the command in the selected PowerShell or Bash session |
| Agent authentication and credentials | Native agent configuration | Never read or copied by Studio |
| Model, endpoint, reasoning, tools, agents, and MCP | Native agent configuration | Not mirrored |
| Sandbox, approval, and permission policy | Native agent configuration and launch flags | Not broadened or auto-approved by Studio |
| Conversation, session, compaction, and structured questions | Native agent TUI | Remain in the terminal |
| Tactus workflow prompt | `AGENTS.md`, `CLAUDE.md`, and `.tactus/agent-instructions.md` | Refreshes an idempotent managed block |

Studio stores no provider session ID, context epoch, permission request, or
raw provider event for the direct workflow.

## Launch boundary

The selected PowerShell or Bash PTY starts in the project root. The user then
chooses a native command:

```powershell
codex
claude
opencode
```

Any model or permission flags belong on that command. Studio does not append a
hidden approval policy. Native login and `requestUserInput`-style interactions
therefore work exactly as supported by the selected CLI.

The terminal host is an interactive surface, not an agent API. Studio does not
parse provider lifecycle messages or promise that equivalent concepts exist
across Codex, Claude Code, and OpenCode.

## Prompt files

`tactus prompt-sync --root .` and the **Sync prompts** button maintain:

```text
.tactus/agent-instructions.md
AGENTS.md
CLAUDE.md
```

The root files contain a delimited managed block. Existing content outside the
block is preserved, and repeated synchronization updates rather than
duplicates it.

`AGENTS.md` is the project instruction entry point for Codex and compatible
agents. `CLAUDE.md` imports that file for Claude Code. The canonical copy below
`.tactus` makes the generated contract easy to inspect.

The managed block includes resolved local paths for the documentation home,
Clef API and entity indexes, Profiles reference, and first-workflow
tutorial when the Clef documentation tree is available. Otherwise it links
to the maintained repository copies. Agents are directed to use those docs and
exported public signatures before inspecting SDK implementation files.

These files are prompts, not enforcement. Studio does not prove that the model
read them, retained them after compaction, or followed them, and a native agent
launched with bypass/full-access permissions can still inspect source files.
There is no context ACK gate or filesystem policy hidden behind prompt sync.

## Formal execution boundary

An agent may use its normal tools for read-only investigation and authoring-time
diagnostics. Formal Tactus execution uses the direct-script lifecycle:

```powershell
tactus script-check --root .
tactus script-run --root .
tactus status --root .
```

or Studio **Check** and **Run**. Material work is first written to a `# %%`
cell in `.tactus/main_script.py`; after execution, the agent inspects the
status, Runtime log, or execution notebook. A failed task is repaired in the
same cell and checked and run again. Ordinary direct commands do not replace
the formal cell result unless the user explicitly requests a direct
command-only action. At formal execution the core runtime composes the current
direct script cell and executes it in a fresh Jupyter kernel. Agent permissions
do not sandbox that kernel. Git and the runtime provide the supported
checkpoint, rollback, repair, and evidence boundary.

## Compatibility CLI

The lower-level `tactus compose`, `discuss`, and `loop` commands may still
launch non-interactive agent processes for compatibility. Their flags and
timeouts apply only to those commands. They are not how production Studio
connects to an agent.

Use the upstream [Codex documentation](https://developers.openai.com/codex/),
[Claude Code documentation](https://docs.anthropic.com/en/docs/claude-code),
and [OpenCode configuration](https://opencode.ai/docs/config/) for native
configuration.
