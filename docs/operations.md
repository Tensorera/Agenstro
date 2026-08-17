---
title: Operate an Agenstro workspace
status: alpha
last_verified: 2026-08-17
applies_to: "Agenstro 0.3"
---

# Operate an Agenstro workspace

This guide covers routine validation, upgrades, backup, diagnostic retention,
Segno recovery boundaries, and safe sharing. It does not turn Agenstro into a
multi-user service or hostile-code sandbox.

## Routine health check

Run from the workspace or pass `--root` explicitly:

```powershell
tactus doctor
tactus list
tactus smoke
```

Use `doctor --json` only for a machine consumer. Human shells should prefer the
four natural-language presentation categories.

For a Segno workspace:

```powershell
segno list --root . --json
segno status --root . --json
```

Plain `smoke` is offline. `smoke --live`, generation, workflow providers, and
some plugins can contact external systems.

## Version control policy

Normally version:

- `.tactus/tactus.toml` after removing machine-local or sensitive values;
- `.tactus/PROMPT.md`;
- `.tactus/scripts`;
- `.tactus/skills` when the project intentionally customizes the bundled
  guidance; and
- project documentation that explains required plugins and provider policy.

Normally ignore:

- `.tactus/runs`;
- `.tactus/path-effect`;
- `.tactus/dist-newstyle`;
- `.tactus/segno/state`;
- provider credentials and session files; and
- generated build output.

The repository's own `.gitignore` is not automatically copied into every
target project. Establish the target project's policy before the first live
run.

## Backup

Stop active Tactus commands and the Segno driver before taking a consistency
backup. Then preserve at least:

```text
.tactus/tactus.toml
.tactus/cabal.project
.tactus/PROMPT.md
.tactus/scripts/
.tactus/skills/
.tactus/segno/jobs/
.tactus/segno/triggers/
.tactus/segno/state/business.sqlite3
.tactus/segno/state/lifecycle.sqlite3
```

The two Segno databases are separate by design. Copy both from the same stopped
workspace snapshot. A business-state checkpoint and lifecycle transition are
not a cross-database exactly-once transaction.

Run journals are optional for functional recovery but useful for diagnosis.
They can be large or sensitive; apply an explicit retention policy rather than
assuming Tactus deletes them automatically.

## Restore or move a workspace

After restoring or moving the project:

1. keep `.tactus` attached to the same project root;
2. repair `.tactus/cabal.project` by rerunning `tactus init --sdk PATH` only if
   the file is missing—existing files are preserved—or edit the link carefully;
3. rerun `segno init --root PROJECT --sdk SEGNO_PATH` when the Segno source path
   changed;
4. run `tactus doctor`;
5. run `tactus check` before any workflow execution; and
6. inspect `segno status` before restarting the driver.

Do not copy one project's lifecycle database into another project while
changing job identities. Occurrence IDs, cursors, idempotency keys, attempts,
and fencing tokens are part of the durable meaning.

## Upgrade binaries and source links

Follow [Installation](install.md) for the canonical commands. After an upgrade:

```powershell
tactus --version
segno --version
tactus check --help | Select-String -Pattern '--package'
tactus doctor
tactus check
```

An executable version string alone is insufficient when testing an untagged
source-alpha commit. Verify the expected command surface and retain the source
commit used for installation.

## Run-journal retention

Each run ID is an opaque directory name below `.tactus/runs`. Delete only
complete, inactive directories selected by an operator policy—for example,
after exporting an incident record and keeping the most recent N days.

Before deletion:

- confirm no Tactus command is using the workspace;
- inspect whether `summary.json` exists;
- retain records referenced by an unresolved `OutcomeUnknown`;
- avoid following symlinks or junctions; and
- delete exact validated paths, never a broad computed root.

Tactus does not currently expose a user-facing journal-prune command. Manual
retention is therefore an operational responsibility.

## `OutcomeUnknown` procedure

When a provider or effect may have acted but no valid terminal result exists:

1. stop automatic retries for that occurrence;
2. record the run ID, operation, external idempotency key, and observed time;
3. inspect the external provider or target system;
4. inspect workspace changes and Tactus diagnostic evidence;
5. for Segno, inspect `segno status` and `segno history`;
6. decide whether the external result can be adopted, compensated, or retried;
   and
7. document the decision outside the immutable run journal.

Segno `0.3` intentionally has no force-success/force-retry mutation command.
Do not edit lifecycle SQLite rows by hand as a substitute.

## Share diagnostics safely

Tactus summarizes recognized prompt, provider, stderr, terminal-value, and
error-detail fields, but diagnostic artifacts are not guaranteed anonymous.
Before attaching them to a report, remove or review:

- home and workspace paths;
- file and document names;
- model/account/organization identifiers;
- timestamps and hashes;
- bounded unknown event fields;
- workflow stdout/stderr; and
- any copied native provider output.

Never share `tactus runtime-json`; it contains resolved instructions and
machine-local command information. Prefer `tactus doctor --json`, `tactus list
--json`, or the redacted Studio control projection.

## Security and credentials

Native providers authenticate through their own tools. Keep credentials out of
TOML, source, prompts, tests, and Git history. Rotate a credential if it enters
a model transcript or commit; removing the latest file revision is not enough.

Configured programs run with the user's operating-system authority. Use a
separate OS account, VM, or container when stronger isolation is required.
Read the repository `SECURITY.md` for the supported claims and reporting path.

## Release verification

Before publishing a source revision:

1. ensure the worktree contains no secrets or generated build output;
2. verify Cargo/Cabal/npm package metadata identifies `AGPL-3.0-only`;
3. run the gates in `CONTRIBUTING.md`;
4. build documentation with `python -m mkdocs build --strict`;
5. confirm installation and upgrade commands against clean user directories;
6. check the support matrix and changelog; and
7. tag the exact commit only after CI succeeds.

The AGPL applies to the repository code. Third-party providers, compilers,
Electron, Node packages, and other dependencies retain their own licenses.
