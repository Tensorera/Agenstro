# Tactus command patterns

Use argument arrays or correctly quoted shell arguments. Keep each path as one argument; paths may contain spaces or non-ASCII characters.

## Discover and diagnose

```powershell
$workspace = (Resolve-Path 'D:\work\project').Path
tactus list --root $workspace --json
tactus doctor --root $workspace --json
```

Tactus searches the supplied path and its ancestors for `.tactus/tactus.toml`. A valid workspace also has `.tactus/scripts`, `.tactus/runs`, and its generated Cabal project. `list --json` returns the resolved workspace and ordered script inventory. `doctor --json` returns `ok` and individual checks; preserve its nonzero exit code when any check fails.

## Inspect and answer decision sessions

```powershell
tactus session list --root $workspace --limit 50
tactus session show --root $workspace --session session-7f3a91
tactus session answer --root $workspace `
  --session session-7f3a91 `
  --turn 3 `
  --axis desk.frame `
  --option fixed `
  --note 'Prefer repairable joinery'
```

Session commands return one `tactus.control/v1` envelope. Treat `turn` as a
compare-and-set token: on `session_turn_stale` or an already-consumed turn,
fetch the current session and ask the person again rather than retrying the old
answer. Tactus currently has no `session advance`; do not invent a planner
invocation command.

## Select scripts

`check` accepts explicit scripts as positional arguments. Paths are interpreted relative to the resolved workspace root unless absolute:

```powershell
tactus check --root $workspace '.tactus\scripts\020_implement.hs'
tactus check --root $workspace `
  '.tactus\scripts\020_implement.hs' `
  '.tactus\scripts\Support.hs'
```

Without explicit paths, `check` selects every `.hs` and `.lhs` source below `.tactus/scripts`. Avoid that broader default when the task names specific files.

`run` uses a repeatable `--script`; it does not accept scripts as bare positionals:

```powershell
tactus run --root $workspace `
  --script '.tactus\scripts\020_implement.hs'
```

Place workflow arguments after `--`:

```powershell
tactus run --root $workspace `
  --script '.tactus\scripts\020_implement.hs' `
  -- '--workflow argument'
```

Without `--script`, `run` executes every numbered `NNN_slug.hs` or `.lhs` entry in Tactus order. Never use that default to validate a single edit.

## Timeouts and packages

Both commands accept `--timeout-seconds N`; the default is 1800 and `0` disables the direct Tactus deadline. Prefer a finite deadline. Repeat `--package NAME` only for required extension libraries. Use `--keep-going` only when the user needs a complete independent-failure inventory.

Exit zero means the selected local command completed successfully. A nonzero check means compilation failed. A nonzero run or missing terminal response may require evidence reconciliation before any retry.
