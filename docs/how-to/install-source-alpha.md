---
title: Install the source alpha
status: alpha
owners: [release]
last_verified: 2026-08-01
applies_to: "0.2.0 source checkout"
platforms: [windows, ubuntu]
---

# Install the Source Alpha

> Historical 0.2 guide. Run these commands only from checkout `c679f45`; the
> current Haskell/Tactus installation is documented in `docs/getting-started.md`.

## Final State

You will have repository-local Python environments and Node dependencies able
to run offline contract checks. You will not have a signed installer, registered
service, authenticated daemon network, or production desktop deployment.

Dependency installation may contact Python or npm registries and incurs their
normal download cost. The verification commands themselves do not contact a
model provider. Run as a normal user; administrator/root access is unnecessary.

## Prerequisites

- Git and a checkout of this exact source version.
- Rust stable plus the declared Rust 1.88 MSRV when testing both routes.
- CPython 3.12 for `clef-sdk` and `tactus-runtime`; Python 3.11 or 3.12 for
  `segno-flow`.
- Node.js 22.12 or newer for `motivo-studio`.
- A local filesystem for SQLite/CAS tests. Do not place state on a UNC share.

No command below changes provider credentials or global Git configuration.
Virtual environments and `node_modules` remain below the checkout.

## Windows PowerShell

Run from the repository root:

```powershell
py -3.12 -m venv .venv-clef
py -3.12 -m venv .venv-tactus
py -3.12 -m venv .venv-segno
.\.venv-clef\Scripts\python.exe -m pip install -e ".\clef-sdk[dev]"
.\.venv-tactus\Scripts\python.exe -m pip install -e ".\tactus-runtime[dev,jupyter]"
.\.venv-segno\Scripts\python.exe -m pip install -e ".\segno-flow[dev]"
npm --prefix motivo-studio ci
```

Then verify public identity and the offline slice:

```powershell
.\.venv-clef\Scripts\python.exe -c "import clef_sdk; print(clef_sdk.__version__)"
.\.venv-tactus\Scripts\tactus.exe --version
.\.venv-segno\Scripts\segno-flow.exe --version
python scripts/generate_integration_fixture.py --check
python scripts/check_workspace_boundaries.py
cargo test -p agentrod --test cross_language_vertical --locked --offline
npm --prefix motivo-studio test
```

The three version checks must print `0.2.0`; all other commands must exit `0`.

## Ubuntu Bash

Ubuntu is a documented source contract but was not run in the current local
integration evidence. Use it as a verification procedure, not a support claim:

```bash
python3.12 -m venv .venv-clef
python3.12 -m venv .venv-tactus
python3.12 -m venv .venv-segno
./.venv-clef/bin/python -m pip install -e './clef-sdk[dev]'
./.venv-tactus/bin/python -m pip install -e './tactus-runtime[dev,jupyter]'
./.venv-segno/bin/python -m pip install -e './segno-flow[dev]'
npm --prefix motivo-studio ci
./.venv-clef/bin/python -c 'import clef_sdk; print(clef_sdk.__version__)'
./.venv-tactus/bin/tactus --version
./.venv-segno/bin/segno-flow --version
python3 scripts/generate_integration_fixture.py --check
python3 scripts/check_workspace_boundaries.py
cargo test -p agentrod --test cross_language_vertical --locked --offline
npm --prefix motivo-studio test
```

If a command fails, retain its output and classify the failure as toolchain,
dependency, platform, or source-contract failure. Do not disable limits or use
global package installation as a workaround.

## Installed and State Locations

| Content | Location in this procedure |
| --- | --- |
| Python SDK/worker environments | `.venv-clef`, `.venv-tactus`, `.venv-segno` |
| Electron/Node dependencies | `motivo-studio/node_modules` |
| Electron package output | `motivo-studio/out` after `npm run build` |
| Rust build cache | inherited `CARGO_TARGET_DIR`, otherwise Cargo default |
| Daemon SQLite/CAS | Not installed; tests use temporary or explicit state roots |

Do not place a Tactus CAS below the workspace it snapshots. The source alpha
does not define a supported per-user production state directory.

## Roll Back This Installation

Stop running tests or Electron processes, then follow the source uninstall
section in [Operate the source alpha](operate-source-alpha.md). Removing these
local dependency directories does not delete explicit daemon state roots or
user workspaces.
