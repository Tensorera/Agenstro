---
title: Run the alpha contract smoke
status: alpha
owners: [release]
last_verified: 2026-08-01
applies_to: "0.2.0"
platforms: [windows]
---

# Run the Alpha Contract Smoke

> Historical 0.2 guide. It requires checkout `c679f45`; the current tree no
> longer contains the `agentrod` or `tactus-core` Rust packages.

This tutorial reproduces the implemented offline path for `clef-sdk`,
`tactus-runtime`, `motivo-studio`, and `segno-flow`. It does not start a daemon
listener or contact a provider.

## 1. Check Public Identity and the Fixture

From the repository root:

```powershell
python scripts/generate_integration_fixture.py --check
python -m unittest discover -s scripts/tests -v
python scripts/check_workspace_boundaries.py
```

Success is exit code `0`. The fixture check is silent and every discovered unit
test must pass. The boundary check prints a success summary for 12 Rust members,
four distributions/entries, and three Python imports.

## 2. Run the Rust Slice

Preserve any inherited `CARGO_TARGET_DIR` and run:

```powershell
cargo test -p agentrod --test cross_language_vertical --locked --offline
```

Expected terminal summary:

```text
test result: ok. 1 passed; 0 failed
```

The test maps the Python-produced JSON fixture through generated Protobuf DTOs,
an in-process Clef service, a Tactus fake worker with real SQLite and external
CAS paths, and an idempotent Segno occurrence dispatch.

## 3. Check Motivo Contracts

If `motivo-studio/node_modules` is installed from its lockfile:

```powershell
npm --prefix motivo-studio run generate
npm --prefix motivo-studio run typecheck
npm --prefix motivo-studio test
```

The tests consume the same workflow DTO and verify the Electron security and
bounded IPC/PTY contracts. Packaging is optional for this tutorial and is not
an installer test.

## Result and Boundary

You have reproduced the in-process contract slice. You have not proven
authenticated loopback gRPC, bundled daemon launch, Python generated stubs,
provider execution, installation, upgrade, Linux GUI behavior, or daemon crash
recovery. See [Known limitations](reference/known-limitations.md).
