# Tactus resource policy

Git is Tactus's rollback mechanism for tracked files. Resource snapshots
observe non-Git files but do not provide byte recovery.

| Cell outcome | Git-tracked files | New allowlisted source/config | Non-Git or large resource |
| --- | --- | --- | --- |
| Success | Committed | Non-ignored files within size/path limits are committed | Other files remain outside Git |
| Failure | Restored to pre-cell commit | Untracked additions are preserved and reported because ownership is ambiguous | Pre-existing untracked content is preserved but cannot be byte-restored |
| Host exit | Reconciled or restored by recovery | Reconciled with the same Git boundary | Reported when outside Git |

## Current in-place project mode

Studio uses the exact project directory. Before a cell, changes to already
tracked files and new allowlisted source/configuration files are committed as a
baseline. The allowlist is configured independently from resource scanning by
`git_track_extensions`; data-oriented `.txt`, `.csv`, and `.tsv` files are not
included by default. New files must also fit `max_auto_track_bytes` and, on
Windows, the conservative `git_track_max_path_chars` limit.

On failure Tactus restores the baseline. It never runs an unscoped
`git clean`: the cell boundary records pre-existing untracked files for
comparison, but never assumes a newly observed path belongs to the cell. New,
renamed, and concurrently created untracked paths are retained. If untracked
content was added, modified, or deleted during the cell, Git has no original
bytes or ownership evidence, so Tactus emits a non-blocking
manual-inspection warning. Empty directories are not versioned or scanned:
Git cannot represent them, and a full-worktree directory walk would make cell
startup unbounded on large dependency trees.

That comparison is deliberately bounded: it records at most 4,096 untracked
entries, hashes at most 16 MiB in total, and collapses wholly untracked
directories to directory-level evidence. Exceeding a bound never blocks a
cell; the rollback warning marks the remaining content as unverifiable.

The project's ordinary ignore policy is respected. Durable Tactus records
are force-added selectively even if a broad project rule ignores
`.tactus`; volatile SQLite, logs, sessions, locks, execution output, and the
Artifact workspace view are never force-added.

This transaction restores the original branch, HEAD, index, and worktree. It
does not promise to reverse arbitrary Git metadata commands performed inside a
cell: for example, a cell may advance another local branch, create a tag,
change a submodule's own repository, or contact a remote. Those operations
remain in Git history or external state. Cells should edit project content and
leave commit, branch, tag, submodule, and remote operations to the host or user.
Tactus refuses to begin or checkpoint a cell while Git reports an active
merge, rebase, cherry-pick, revert, bisect, sequencer, or unmerged index.

Edits made between cells to tracked or allowlisted files are included safely in
the next pre-cell baseline. Additions that exceed the size/path policy remain
outside Git; their bounded skipped-path summary is exposed in runtime status
and, during first-time Git initialization, the Studio lifecycle log.

## Resource observation

Resource scanning excludes control, environment, and cache directories.
Files at or below `resource_hash_limit_bytes` can be content-hashed. Larger
resources use path metadata for change observation.

Use Git, a content-addressed store, or an external versioned data system when
byte recovery is required.

## Imported resource isolation

This section applies only to legacy detached runtimes created with
`tactus init --source --root`. New Studio projects already operate where
their resources live and do not import the project into another worktree.

In compatibility mode, mutable imports are copied into the worktree. An import marked immutable may
use `auto` or `hardlink_readonly`:

- only ordinary files on a fixed local Windows volume are eligible;
- a same-file identity, size, modification time, and SHA-256 digest are
  recorded outside the worktree;
- Windows share-deny-write/delete handles protect both source and hardlink
  alias for the complete host lifetime;
- reopening a runtime reacquires those handles and rejects any identity or
  digest drift before an agent or cell may run; and
- `auto` records why it fell back to a copy.

Symbolic links, junctions, reparse points, and special files are rejected.
Framework control namespaces are excluded by ownership. A project file is not
excluded merely because it is named `task.json`.

The hardlinks under `.tactus/Artifact/workspace` serve a different purpose:
they are a browsable projection of accepted artifact history, not protected
input isolation and not rollback storage.
