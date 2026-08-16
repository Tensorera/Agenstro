# Tactus configuration reference

Tactus has two configuration scopes:

1. `~/.tactus/settings.json` contains user defaults only. It must not contain
   project runtime state, cells, logs, sessions, or task results.
2. `<project>/.tactus/config.json` describes the exact project. Its
   `settings` recursively override user defaults.

This split leaves room for future agent/model fields such as model choice or
reasoning level without moving execution data into the user's home directory.

## Project configuration

Studio writes the project file when it first opens a directory. Existing files
are loaded in place.

| Field | Type | In-place value/default | Meaning |
| --- | --- | --- | --- |
| `schema_version` | integer | `1` | Configuration schema version |
| `tactus_id` | string | Generated UUID hex | Stable project identity |
| `root` | absolute path | Studio startup directory | Exact project root |
| `source_repository` | absolute path | Same as `root` | Compatibility field |
| `workspace_mode` | string | `in_place` | Agents and cells use the project directly |
| `worktree` | relative path | `.` | Execution workspace relative to `root` |
| `notebook` | relative path | `.tactus/main-thread.ipynb` | Rebuildable Notebook projection |
| `database` | relative path | `.tactus/state.sqlite3` | SQLite state authority |
| `control_directory` | relative path | `.tactus` | Tactus-owned namespace |
| `helper_directory` | relative path | `.tactus/helpers` | Helper scripts |
| `kernel_name` | string | `python3` | Cell kernelspec |
| `timeout_seconds` | positive integer | `900` | Cell execution timeout |
| `max_inline_output_chars` | integer | `65536` | Inline output retention limit |
| `max_auto_track_bytes` | positive integer | `4194304` | Maximum size of a newly allowlisted file automatically staged by Tactus |
| `git_track_extensions` | string array | Source/config suffix allowlist | New files eligible for Git cell checkpoints |
| `git_track_max_path_chars` | integer | `240` | Conservative Windows absolute-path limit for automatic Git staging |
| `settings` | object | `{}` | Project overrides for user defaults |
| `created_at` | timestamp string | Creation time | Project creation record |

Additional resource-policy fields may also be present in schema v1. Some,
including `max_auto_track_bytes`, apply to in-place Git policy; detached-only
fields do not make Studio create a detached workspace.

`git_track_extensions` is deliberately separate from `text_extensions`, which
belongs to resource inspection. The default Git allowlist includes Markdown
and common programming, build, and configuration formats; it excludes
data-oriented `.txt`, `.csv`, and `.tsv` files. Existing files already tracked
by the user's repository remain tracked regardless of suffix.

## User configuration

A minimal optional user file is:

```json
{
  "schema_version": 1,
  "settings": {
    "agents": {
      "codex": {
        "model": "configured-by-user"
      }
    }
  }
}
```

Project overrides can be narrower:

```json
{
  "settings": {
    "agents": {
      "codex": {
        "reasoning": "high"
      }
    }
  }
}
```

Tactus recursively merges nested settings and gives the project value
precedence. Coding-agent native configuration remains authoritative unless a
future script or integration explicitly consumes one of these settings.

## Git is not configured here

When the exact project root is not a Git repository, Studio asks whether to
run `git init` there. It does not edit local or global Git settings. Tactus
uses command-scoped identity options for its own commits, so it does not need
to overwrite `user.name`, `user.email`, signing, hooks, remotes, branches, or
ignore policy.

## Timeout scopes

| Interface | Default | Scope |
| --- | --- | --- |
| Project `timeout_seconds` | 900 seconds | One Jupyter cell |

Production Studio does not impose a coding-agent turn timeout because the
native agent runs interactively in the selected xterm.js PowerShell or Bash
session.
Compatibility commands such as `tactus compose --agent` and
`tactus loop` retain their own command-line timeout flags.

## Legacy compatibility

The schema still recognizes `workspace_mode = "detached"`, `worktree =
"workspace"`, external resource import fields, and separate source/root paths
for older CLI-created runtimes. Those fields describe compatibility mode, not
the current Studio default.
