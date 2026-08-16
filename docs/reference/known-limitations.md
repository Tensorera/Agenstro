---
title: Known limitations
status: alpha
owners: [release]
last_verified: 2026-08-01
applies_to: "0.2.0"
platforms: [windows, ubuntu]
---

# Known Limitations

> Historical 0.2 record for checkout `c679f45`. It does not describe the
> current Haskell Clef and Python Tactus implementation.

These limitations are part of the `0.2.0` acceptance boundary.

## No Shipped Daemon Network

The repository has in-process `agentrod`, `TactusDaemon`, and `Segnod`
compositions plus generated protocol DTOs. It does not ship authenticated gRPC
listeners, handlers, discovery, or the bootstrap pipe implementation expected
by production clients. Python `script-run` and daemon-facing Segno commands
therefore require unavailable injected transports.

## Three-Database Contract Is Incomplete

The architecture assigns one private SQLite database to each of `agentrod`,
`tactusd`, and `segnod`; daemons must not open each other's files. Tactus opens
an explicit database path and Segno opens `segno.sqlite3` below its explicit
state root. Current `agentrod` retains plans and runs in bounded memory and has
no SQLite repository. The required third database and restart recovery are not
implemented.

## Motivo Is Degraded Without Daemons

Motivo's Electron main/preload/renderer security boundary and Windows package
build are implemented. Forge currently bundles `node-pty`, not daemon
binaries. Startup catches daemon bootstrap failure and exposes `agentrod`,
`tactusd`, and `segnod` as unavailable. No installer, signing, auto-update, or
service discovery is delivered.

## Worker Integration Is Split

Rust tests validate bounded worker framing and process supervision. Python tests
validate the ordinary and Jupyter worker processes. The final generated
Protobuf payload adapter connecting Rust framing to
`python -m tactus_runtime.worker` is not complete.

## Restore Is Conservative

The external CAS must be outside the mutable workspace. Capture is bounded and
can represent a full included manifest, but automatic restore considers at
most caller-declared regular files whose current bytes still match an observed
checkpoint. It does not delete new files, restore symlinks, or overwrite an
externally changed path. CAS retention and garbage collection are not shipped.

## No Release Operations

There are no signed Windows or Ubuntu installers, system/user services,
checksums, SBOM, release manifest, backup command, updater, downgrade tool, or
data purge command. Source removal is not equivalent to data deletion. Database
schema compatibility with a previous release has not been tested.

## Platform and Scale Gaps

The current integration evidence is Windows-local. Ubuntu is represented in CI
configuration but was not executed for the integration report. Linux GUI,
WSL, UNC/network filesystems, 100k/1m file workloads, long soak, disk-full,
real daemon restart, and install/upgrade rollback matrices remain unverified.

## No Production Provider Integration

Adapter manifests and fake transcript conformance do not launch providers,
read credentials, or use the network. No Codex, Claude Code, OpenCode, or ACP
version is authenticated, protocol-ready, or live-tested by this alpha slice.

## Migration Is Narrow

Only prototype Clef workflow JSON has a one-way v1-to-v2 converter. There is no
Tactus prototype database/notebook importer and no Segno prototype registry or
run-history importer. See the [migration guide](../migrations/prototype-to-greenfield.md).
