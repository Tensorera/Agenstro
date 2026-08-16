---
title: Operate the source alpha
status: alpha
owners: [release, runtime]
last_verified: 2026-08-01
applies_to: "0.2.0 source checkout"
platforms: [windows, ubuntu]
---

# Operate the Source Alpha

> Historical 0.2 guide. Its package counts and `agentrod` commands apply only
> to checkout `c679f45`, before the Clef/Tactus Rust product cores were removed.

This guide covers offline doctor checks and conservative source upgrade,
rollback, and uninstall. The `0.2.0` alpha has no installed `doctor`, updater,
backup, rollback, or uninstall command. The procedures below use implemented
version/check/test surfaces and keep user/provider state untouched.

## Run Offline Doctor Checks

The final state is a machine-readable version result plus green source-contract
checks. These checks do not start daemons, open production SQLite, scan user
home, read provider credentials, or use the network.

### Windows PowerShell

Run from the repository root:

```powershell
$ErrorActionPreference = "Stop"
.\.venv-clef\Scripts\python.exe -c "import clef_sdk; print(clef_sdk.__version__)"
.\.venv-tactus\Scripts\tactus.exe --version
.\.venv-segno\Scripts\segno-flow.exe --version
python scripts/generate_integration_fixture.py --check
python scripts/check_workspace_boundaries.py
cargo metadata --no-deps --locked --offline --format-version 1 |
  ConvertFrom-Json |
  Select-Object -ExpandProperty packages |
  Measure-Object |
  Select-Object -ExpandProperty Count
```

Expected version lines are `0.2.0`, `tactus 0.2.0`, and `segno-flow 0.2.0`.
The fixture check is silent, the boundary script exits `0`, and Cargo reports
12 workspace packages.

### Ubuntu Bash

```bash
set -eu
./.venv-clef/bin/python -c 'import clef_sdk; print(clef_sdk.__version__)'
./.venv-tactus/bin/tactus --version
./.venv-segno/bin/segno-flow --version
python3 scripts/generate_integration_fixture.py --check
python3 scripts/check_workspace_boundaries.py
cargo metadata --no-deps --locked --offline --format-version 1 \
  | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["packages"]))'
```

Ubuntu results remain `Contract only` until the complete matrix runs in CI or
a recorded VM. Do not relabel a source parse as platform support.

### Interpret Failures

| Symptom | Meaning | Action |
| --- | --- | --- |
| Import or entry missing | Wrong/missing local virtual environment | Reinstall that project into its own environment |
| Fixture drift | Python builder and checked-in fixture disagree | Regenerate only as part of a reviewed protocol change |
| Boundary failure | Public name/version or Rust dependency direction changed | Restore the documented public identity or review the architecture change |
| Cargo offline dependency failure | Required crate is not cached | Use an approved dependency-fetch step; do not remove `--locked` |
| Motivo shows all daemons unavailable | Expected current package behavior | Do not substitute mock services; matching binaries/listeners are not shipped |
| `script-run` or Segno daemon command returns unavailable | Generated authenticated transport is absent | Use only offline authoring/check surfaces in this alpha |

There is no safe read-only database doctor command yet. Do not open production
state with internal test binaries merely to inspect it: `segnod` startup may
create directories and apply schema v1.

## Upgrade a Source Checkout

No database upgrade path has release support. Use side-by-side source and state
instead of modifying the only copy.

1. Stop Motivo, worker processes, and any manually started `segnod` composition.
2. Record the old commit, `rustc --version`, Python versions, and Node version.
3. Copy every explicitly supplied Tactus/Segno state root, including SQLite
   `-wal`/`-shm` files and the complete external CAS/package tree, while all
   owners are stopped.
4. Create a separate checkout for the new source. Do not overwrite the old
   virtual environments or `node_modules`.
5. Follow [Install the source alpha](install-source-alpha.md) in the new
   checkout and run the full offline smoke.
6. Use only temporary new state for runtime tests. Current evidence does not
   authorize opening old state with a new binary as a supported migration.

The source build neither reads nor modifies Codex, Claude Code, OpenCode, Git
global configuration, shell profiles, or global Python/Node installations.

## Roll Back

1. Stop every process from the new checkout.
2. Preserve the new checkout and any new state for diagnosis; do not merge its
   SQLite or CAS files into the old state.
3. Start the old checkout with the untouched pre-upgrade state copy.
4. Re-run the old version and boundary checks before allowing writes.

Binary rollback against a database already opened by a newer schema is not
supported. Restore the complete stopped-owner backup instead. There is no
reverse migration and no authority to infer one from `PRAGMA user_version`.

## Uninstall the Source Checkout

Source uninstall removes only repository-local dependencies and generated
packages. First inspect the exact targets.

### Windows PowerShell

```powershell
Get-Item .\.venv-clef, .\.venv-tactus, .\.venv-segno,
  .\motivo-studio\node_modules, .\motivo-studio\out -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force .\.venv-clef, .\.venv-tactus, .\.venv-segno -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force .\motivo-studio\node_modules, .\motivo-studio\out -ErrorAction SilentlyContinue
```

### Ubuntu Bash

```bash
find . -maxdepth 2 -type d \
  \( -name '.venv-clef' -o -name '.venv-tactus' -o -name '.venv-segno' \
     -o -path './motivo-studio/node_modules' -o -path './motivo-studio/out' \) -print
rm -rf -- .venv-clef .venv-tactus .venv-segno \
  motivo-studio/node_modules motivo-studio/out
```

Do not delete an inherited `CARGO_TARGET_DIR`: it may be outside the checkout
and shared by other work. Removing source dependencies does not delete user
workspaces or explicitly configured SQLite/CAS/package state.

## Data Purge

No data-purge command exists. Keep data by default. If a test-only state root
must be removed, first stop its owner and verify its canonical absolute path is
the disposable root you created. Never treat a workspace, home directory,
drive root, or provider configuration directory as disposable state.
