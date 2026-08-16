---
title: Runtime boundaries
status: alpha
owners: [foundation, runtime, desktop]
last_verified: 2026-08-01
applies_to: "0.2.0"
platforms: [windows, ubuntu]
---

# Runtime Boundaries

The greenfield contract separates orchestration, execution, scheduling, and UI
ownership. A shared database or a renderer-owned credential would collapse
failure and security boundaries.

## Intended Process and State Topology

```text
Motivo renderer (no Node, no token)
  -> narrow preload API
Electron main
  -> agentrod  -> private Clef SQLite (required; not implemented)
  -> tactusd   -> private Tactus SQLite + external SHA-256 CAS
  -> segnod    -> private Segno SQLite + immutable package store
                     |
                     +-> idempotent occurrence dispatch to agentrod

tactusd -> supervised external Python worker or optional Jupyter worker
```

The three daemons must own three separate databases and must not open each
other's database files. Cross-service state moves through versioned protocol
values and stable references. CAS lives under service state outside the mutable
workspace; neither Git objects nor a project-local `.tactus` directory is its
authority.

## Current Alpha Reality

| Boundary | Implemented | Degraded or absent |
| --- | --- | --- |
| `agentrod` | Bounded in-process plans, runs, events, cancellation, normalized backend port | SQLite, restart recovery, listener and provider host |
| `tactusd` composition | Private SQLite actor, fences/leases, external CAS, bounded worker/process paths, conservative restore | Daemon executable/listener and final real-Python generated payload adapter |
| `segnod` composition | Private `segno.sqlite3`, immutable revisions, cron/DST policy, leases/outbox, idempotent dispatch reference | Authenticated service/discovery and long-running daemon loop |
| Motivo | Electron main/preload/renderer split, secure web preferences, schema-checked IPC, owned PTY utility process | Bundled daemons, `segnod` discovery, installer and production E2E |

Motivo therefore reports an explicit degraded snapshot rather than substituting
a mock transport.

## Python Worker Boundary

Python executes trusted user code; it does not own durable scheduling or
workspace state. Worker stdout is reserved for bounded protocol frames. Tactus
validates frame version, run identity, contiguous sequence, lifecycle order,
per-frame size, per-chunk size, total output, deadlines, and cancellation. The
process supervisor owns tree cleanup. These controls are reliability limits,
not an adversarial code sandbox.

## Electron Security Boundary

The BrowserWindow enables context isolation and sandboxing while disabling Node
integration, webviews, navigation, window creation, and permissions. Preload
exposes named methods with input/output schemas; it does not expose
`ipcRenderer`, arbitrary paths, process creation, or bearer tokens. Electron
main creates 256-bit bootstrap tokens in memory and zeroes them during cleanup.

These controls reduce renderer authority. They do not make a trusted Python
worker or PTY shell safe to run hostile code.

## Failure Ownership

- `agentrod` owns workflow/task terminal state and closes normalized sessions.
- `tactusd` owns execution cancellation, process cleanup, output publication,
  checkpoint state, and worker shutdown.
- `segnod` owns occurrence uniqueness, lease fences, outbox intent, and only an
  orchestration run reference plus bounded terminal summary.
- Electron main owns desktop child processes and closes PTYs and bootstrapped
  daemons before application exit.

No queue, stream, list, worker, or child process in the implemented slice is
documented as unbounded.
