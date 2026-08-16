# Tactus CLI reference

The `tactus` executable exposes one operation per subcommand. Commands emit
JSON unless they start the graphical Studio.

The current interactive entry point is:

```powershell
Set-Location D:\Projects\my-project
motivo-studio
```

`tactus studio` is equivalent. Studio loads or initializes only the exact
startup current directory and asks about `git init` when that directory is not
an independent Git root. It opens the built React + TypeScript/Vite interface
in a local pywebview window. Monaco provides the editor, while xterm.js uses
pywinpty/ConPTY for a project-root PowerShell or Bash session. Windows Terminal
is not required. Studio projects `.tactus/main_script.py` and does not use
the `init` command below.

## Commands

| Command | Purpose |
| --- | --- |
| `init` | Create a legacy detached-worktree runtime root |
| `doctor` | Read configuration and state health without advancing the workflow |
| `script-check` | Compile the direct Studio script without executing it |
| `script-run` | Run pending direct-script cells through the core runtime |
| `prompt-sync` | Refresh managed Tactus blocks in agent instruction files |
| `compose` | Create a draft cell from code or an agent decision |
| `run` | Execute the latest draft cell |
| `message` | Append durable human steering for a later compose |
| `discussion-start` | Enter persistent Discussion mode |
| `discuss` | Send one human/agent Discussion turn |
| `discussion-end` | Leave Discussion mode and return to Compose |
| `interactions` | List durable human/agent messages |
| `interrupt` | Request a graceful or immediate task stop |
| `resume` | Resume compose/run progression after a completed stop |
| `loop` | Alternate agent compose and run operations |
| `status` | Return current workflow state |
| `recover` | Reconcile or roll back a stale running cell |
| `warnings` | List or acknowledge user-facing warnings |
| `migrate` | Explicitly migrate a v1 state database |
| `rebuild-notebook` | Rebuild `main-thread.ipynb` from SQLite |
| `studio` | Open or initialize Studio in the exact current directory |

`script-check`, `script-run`, and `prompt-sync` are the production Studio
command surfaces. `compose`, `discuss`, `loop`, and `interrupt` remain
lower-level or compatibility interfaces; production Studio does not use them
to manage an agent or coordinate multiple cells.

## Direct Studio commands

```text
tactus script-check [--root PATH]
tactus script-run [--root PATH]
tactus prompt-sync [--root PATH]
```

`--root` defaults to the current directory.

`script-check` parses `.tactus/main_script.py`, compiles the complete file,
and compiles every executable `# %%` cell without executing it.

`script-run` performs the same validation, then runs pending direct cells in
source order through `Tactus.compose()` and `run_latest()`. It stops at the
first failure. Exit code `0` means all current cells succeeded or were already
up to date; exit code `1` means a cell failed. Every terminal attempt appends a
manual continuation record under `.tactus/continuations/`.

`prompt-sync` writes the canonical
`.tactus/agent-instructions.md` and idempotently updates managed blocks in
root `AGENTS.md` and `CLAUDE.md`. User text outside the managed blocks is
preserved.

## `init`

`init` is retained for compatibility and explicit detached runtimes. It is not
the default Studio bootstrap path.

```text
tactus init --source PATH --root PATH
  [--revision REF]
  [--include GLOB]...
  [--resource SOURCE[=DESTINATION]]...
  [--immutable-resource SOURCE[=DESTINATION]]...
  [--hardlink-resource SOURCE[=DESTINATION]]...
  [--hardlink-min-bytes BYTES]
  [--timeout SECONDS]
  [--kernel NAME]
```

`--revision` defaults to `HEAD`. `--timeout` is the Jupyter cell execution
timeout and defaults to 900 seconds. `--kernel` defaults to `python3`.
`--resource` copies mutable data. `--immutable-resource` uses protected
same-volume hardlinks for files at least 64 MiB by default and otherwise
copies. `--hardlink-resource` requires protected hardlinks and fails closed if
they are unavailable.

## `compose`

Manual code mode requires exactly one of `--file` or `--code`:

```text
tactus compose --root PATH (--file PATH | --code TEXT)
```

Agent mode requires `--agent`:

```text
tactus compose --root PATH --agent {codex,claude,opencode}
  [--objective TEXT]
  [--instruction TEXT]
  [--timeout SECONDS]
```

The timeout limits one Codex, Claude Code, or OpenCode process and defaults to
300 seconds.
Objective becomes durable workflow state when supplied for the first compose.
It becomes immutable after the first cell exists or the workflow completes.
Instruction applies to the current request.

## `run`

```text
tactus run --root PATH
```

Only the newest draft cell can run.

The JSON result contains `cell`, `warnings`, and `interrupt`. `interrupt` is
non-null when a human stop request completed with that run. In that case,
`run` exits with code `130`.

## `message`

```text
tactus message --root PATH --content TEXT
```

`message` appends one immutable user message in Compose mode without invoking
Codex, Claude Code, or OpenCode. The message becomes durable steering in later compose
contexts. Appending a Compose message changes the workflow revision, so an
agent decision that was already in flight cannot commit against stale human
input.

Content must be non-empty and at most 100,000 characters. `message` is rejected
while the runtime is in Discussion mode.

The command returns:

| Field | Meaning |
| --- | --- |
| `message_id` | Stable message identifier |
| `sequence` | SQLite event sequence |
| `role` | `user`, `assistant`, or `system` |
| `content` | Message text |
| `mode` | `compose` or `discussion` |
| `created_at` | Durable creation timestamp |
| `in_reply_to` | Referenced user message ID for a reply, otherwise `null` |

## Discussion commands

```text
tactus discussion-start --root PATH

tactus discuss --root PATH --message TEXT
  [--agent {codex,claude,opencode}]
  [--timeout SECONDS]

tactus discussion-end --root PATH
```

`discussion-start` enters durable Discussion mode. Discussion can start only
between cell executions. Compose and Run are rejected until
`discussion-end` restores Compose mode.

`discuss` requires Discussion mode. The command stores the user message, asks
the selected main agent for a natural-language response, stores the response,
and returns the assistant message object described under [`message`](#message).
The default agent is Codex and the default timeout is 300 seconds.

Each Discussion turn creates no cell. `discuss_with()` gives the agent a
disposable copy of ordinary workspace files, without `.git`, `.tactus`, or
`__pycache__`. A symbolic link, junction, reparse point, or special file makes
the turn fail before the agent starts. Codex Discussion additionally forces
`--sandbox read-only`; OpenCode retains its native permission configuration
inside the disposable copy. This protects the authoritative project but is
not a network or general host sandbox.

The agent receives bounded workflow, workspace, recent-cell, memory, and
interaction context. Multiple `discuss` commands remain in the same persistent
Discussion session until `discussion-end` is called explicitly.

`discussion-start` and `discussion-end` return the current
`interaction_mode`. Repeating either command when already in its requested
mode is idempotent.

## `interactions`

```text
tactus interactions --root PATH [--limit COUNT]
```

`interactions` returns the newest durable messages, ordered chronologically
within the returned window. `--limit` must be positive and defaults to 200.
Each array item uses the message fields documented under
[`message`](#message).

## `interrupt`

```text
tactus interrupt --root PATH
  [--mode {graceful,immediate}]
```

The default mode is `graceful`.

- `graceful` lets the targeted running cell finish through its normal success
  or failure path, then pauses the task. A successful checkpoint remains valid.
- `immediate` asks the executor to stop and invalidate the targeted cell.
  Tactus marks the cell `interrupted`, discards recorded outputs, rolls
  tracked files back, preserves ambiguous untracked resources, and reports
  residue that Git cannot safely restore.

When no cell is running, either mode completes immediately and pauses the task
before another compose or run. Repeating an active request returns the existing
request. An active graceful request can be escalated to immediate while
preserving its `request_id`.

The command returns:

| Field | Meaning |
| --- | --- |
| `request_id` | Stable interrupt identifier |
| `mode` | `graceful` or `immediate` |
| `status` | `requested`, `acknowledged`, or `completed` |
| `cell_id` | Target running cell, or `null` for an idle stop |
| `requested_at` | Request timestamp |
| `acknowledged_at` | Runner-observation timestamp, when present |
| `completed_at` | Timestamp at which the task became paused, when complete |

Immediate invalidation still waits for safe containment cleanup and rollback.
Internal recorded outputs are discarded. Project files are never auto-deleted
when their owner is ambiguous; remaining differences are reported as a warning.

## `resume`

```text
tactus resume --root PATH
```

`resume` changes `task_state` from `paused` to `active` and clears the completed
interrupt request. Resume is rejected while a cell is running or while an
interrupt remains `requested` or `acknowledged`. Calling Resume for an already
active task is idempotent.

Resume does not rerun an invalidated cell. Compose a new same-phase attempt
after an immediate stop. The command returns `{"task_state": "active"}`.

## `loop`

```text
tactus loop --root PATH --objective TEXT
  [--instruction TEXT]
  [--agent {codex,claude,opencode}]
  [--timeout SECONDS]
  [--max-cells COUNT]
```

The default agent is Codex. The agent process timeout defaults to 300 seconds
and `--max-cells` defaults to 20. Resuming an incomplete runtime requires the
exact same objective. Use a new runtime root for a different objective.

`loop` checks `task_state` before composing and after each compose/run
boundary. A completed interrupt or an already paused task stops the loop with
reason `interrupted` and exit code `130`. Run `resume` before starting another
loop.

See [Tactus runtime configuration](tactus-runtime-config.md#timeout-scopes)
for the two independent timeout domains.

## Recovery and state

```text
tactus recover --root PATH [--force]
tactus status --root PATH
tactus warnings --root PATH [--all] [--ack ID]
tactus rebuild-notebook --root PATH
```

`status` includes `snapshot_sequence`, `task_state`, `interaction_mode`,
`interrupt_request`, `pending_interrupt`, `active_cell`, and the durable
`interactions`/`messages` arrays in addition to workflow, phase, recovery, and
warning state. `snapshot_sequence` is the maximum SQLite event sequence read
in the same transaction as the rest of the snapshot.

Every `cells[]` item includes durable identity and status plus `created_at`,
`updated_at`, `started_at`, `finished_at`, `output_count`, bounded
`output_preview`, `output_truncated`, structured `error`, and ordered `events`.
`active_cell.events` exposes the same event stream for the running cell.
Production Studio's Runtime log projects recent cell status, attempt number,
output preview, structured error, and warnings from these durable fields. It
does not maintain a second provider or RunCoordinator state authority.

Stop and recovery are different operations. `interrupt` is a durable human
request handled by a live runner. `recover` claims a cell whose previous owner
ended unexpectedly and reconciles durable execution evidence.

## Migration

```text
tactus migrate --root PATH --confirm-hosts-stopped
```

The confirmation declares that every v1 host using the root has stopped. It
does not terminate processes.

## Doctor

```text
tactus doctor --root PATH
  [--agent {codex,claude,opencode}]
  [--executable PATH]
```

The default agent is Codex. Doctor returns exit code `0` for a healthy report
and `1` for an unhealthy report.

## Process exit codes

| Code | Meaning |
| --- | --- |
| `0` | Command completed successfully |
| `1` | Doctor is unhealthy, a cell run failed, or loop reached `max-cells` before completion |
| `2` | Tactus rejected the operation |
| `130` | A durable stop completed, a paused task stopped `loop`, or the process received `KeyboardInterrupt` |
