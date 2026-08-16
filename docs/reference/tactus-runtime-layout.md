# Tactus project layout

An Tactus project is the exact directory from which `motivo-studio` was
started. The project and execution workspace are the same directory. Tactus
does not search a parent or child directory for `.tactus`, and it does not
create a temporary mirror by default.

```text
<project>/
├── .git/                              # optional, owned by Git
├── AGENTS.md                          # user text + managed Tactus block
├── CLAUDE.md                          # user text + managed import block
├── src/ ...                           # ordinary project content
└── .tactus/
    ├── .gitignore                     # excludes volatile runtime state
    ├── config.json                    # durable project configuration
    ├── main_script.py                 # authoritative direct Python script
    ├── agent-instructions.md          # canonical generated agent prompt
    ├── helpers/                       # Tactus helper scripts
    ├── artifact-tree.json             # logical artifact history
    ├── Artifact/
    │   ├── tasks/                     # accepted task-result envelopes
    │   └── workspace/                 # hardlink/pointer browsing view
    ├── continuations/
    │   ├── events.jsonl               # manual continuation outbox
    │   ├── latest.json
    │   └── latest.md
    ├── state.sqlite3                  # runtime state authority
    ├── main-thread.ipynb              # rebuildable execution projection
    ├── git-control/                   # Git operation lease and empty hooks
    ├── cache/                         # project-local disposable rendezvous
    ├── logs/                          # Studio lifecycle diagnostics
    └── runs/                          # per-attempt execution evidence
```

## Authorities

| Path or system | Purpose | Authority |
| --- | --- | --- |
| Project files | Actual build, debug, and agent workspace | Current filesystem and Git |
| Git | Pre-cell boundary, success commits, tracked-file rollback | Git history |
| `.tactus/config.json` | Project-local settings | Project configuration |
| `.tactus/main_script.py` | Current planned direct cells | The file itself |
| `.tactus/state.sqlite3` | Phases, attempts, cell events, warnings, and outputs | Runtime state |
| `.tactus/main-thread.ipynb` | Human-readable execution history | Rebuildable projection |
| `.tactus/artifact-tree.json` | What accepted tasks reported over time | Artifact history |
| `.tactus/Artifact/tasks/` | Accepted result envelopes | Task evidence |
| `.tactus/Artifact/workspace/` | Convenient artifact browsing view | Rebuildable projection |
| `.tactus/continuations/` | Suggested next-agent prompts | Manual durable outbox |

The direct script is not backed by an approval journal. A script cell is
identified at execution by its ordinal and source digest; durable attempts
remain in SQLite even after the file is edited.

## Artifact view

The Artifact view does not copy the project. A file normally appears as a
hardlink to the real project file. Directories contain hardlinked regular
files. If a hardlink cannot be created, Tactus writes a diagnostic pointer.
Symlinks, junctions, reparse points, and `.tactus` recursion are not
followed.

Task JSON belongs under `.tactus/Artifact/tasks`; a task must not place its
framework result JSON in the project root.

Only a strict `clef_sdk.task-result/v1` envelope mutates the Artifact
Tree. Untagged JSON may be archived as result evidence with zero mutations.
The tree is historical evidence, not the authority for current filesystem
bytes.

## Git tracking

The generated `.tactus/.gitignore` excludes volatile SQLite sidecars, logs,
terminal sessions, locks, `git-control`, cache data, execution output, and the
rebuildable Artifact view. Durable configuration, `main_script.py`, helpers,
artifact metadata, and task results remain eligible for Tactus's Git
allowlist.

Git operation gates, kernel readiness files, and disposable discussion copies
are all created below the project's own `.tactus` directory. Tactus does
not create control or recovery directories beside the project.

Root `AGENTS.md` and `CLAUDE.md` are ordinary project files. Studio preserves
all content outside its delimited managed block.

## Legacy data

Older projects can contain:

```text
.tactus/main-script.md
.tactus/main-script/events.jsonl
.tactus/agents/
.tactus/run-coordinator/
```

The current Studio may read active legacy cells once to seed
`main_script.py`, but these paths are not current authorities. They remain on
disk for compatibility and diagnosis.

Configurations with `workspace_mode = "detached"` and a separate `workspace/`
also remain loadable for older CLI-created runtimes. New Studio startup uses
`workspace_mode = "in_place"`.

Releases before this layout could leave a sibling
`<project-parent>/.tactus-git-control` directory. The current runtime does
not reuse, modify, or remove that legacy sibling. After every older Tactus
process has stopped, it can be reviewed and removed manually.
