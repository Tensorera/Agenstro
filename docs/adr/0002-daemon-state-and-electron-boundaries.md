# ADR 0002: Daemon State and Electron Boundaries

Status: Superseded for Clef/Tactus by ADR-0003; Motivo/Segno design retained

The Clef/Tactus Rust daemons described here exist only in snapshot `c679f45`.
Motivo and Segno source remains in the current tree pending redesign.

Date: 2026-08-01

Owners: Foundation, Runtime, Segno, Desktop

## Context

Clef orchestration, Tactus execution, and Segno scheduling have different
durability, retention, and failure semantics. Motivo needs local access without
giving an Electron renderer filesystem, process, database, or daemon-secret
authority. The prototype combined several of these concerns in Python or UI
state and cannot be translated into the new control plane.

## Decision

1. `agentrod`, `tactusd`, and `segnod` are separate process owners.
2. Each daemon owns one private SQLite database. No daemon opens another
   daemon's database, and cross-daemon links are stable protocol identifiers.
3. Tactus owns one external SHA-256 CAS outside the mutable workspace. Git is a
   read-only metadata adapter; it is not the checkpoint store.
4. User Python and optional Jupyter execution occur in supervised external
   workers with bounded versioned framing, cancellation, deadlines, output, and
   explicit shutdown.
5. Electron main owns daemon bootstrap and PTYs. Preload exposes only named,
   validated operations. The renderer receives no Node API or daemon token.
6. A missing process or transport produces an explicit degraded capability. A
   production mock is not a fallback.

The current alpha implements Tactus and Segno SQLite composition, external CAS,
worker contracts, and the Electron boundary. `agentrod` persistence, daemon
listeners, authenticated discovery, and bundled binaries remain required work.

## Alternatives

- A shared SQLite database was rejected because it permits schema and locking
  coupling and bypasses service authorization.
- Project-local CAS or Git commits as authority were rejected because they
  mutate user control data and cannot provide the same contract for non-Git
  workspaces.
- In-process Python execution was rejected because process ownership,
  cancellation, and interpreter failure cannot be isolated cleanly.
- Renderer-to-daemon or renderer-to-filesystem access was rejected because an
  XSS would inherit durable state and credential authority.

## Consequences

- Installation and startup must coordinate three compatible daemon processes,
  three migrations, one external CAS, worker environments, and Electron.
- Cross-daemon operations require explicit idempotency, failure reconciliation,
  and version negotiation; local SQL joins are forbidden.
- Backups and upgrades are more operationally involved because consistent
  state spans separate owners and CAS references.
- Degraded mode is visible sooner, but some UI features remain unavailable
  until all required transports are shipped.
- Electron sandboxing does not sandbox PTY commands or trusted Python code.

## Compatibility and Migration

The four public project/distribution names, three Python imports, and existing
CLI/GUI entry names remain unchanged. Internal `agentro-*` crates and daemon
names do not introduce a public `agentro` Python package. Prototype databases,
notebooks, registries, and provider configuration are not dual-read. Supported
data conversion is one-way and produces new-format artifacts before any new
daemon import.

## Validation

- Root boundary tests verify names, workspace direction, and generated fixture
  stability.
- Rust tests exercise separate Tactus/Segno SQLite paths, external CAS,
  process ownership, cancellation, fences, and idempotent dispatch.
- Motivo tests exercise secure window preferences, narrow IPC, bootstrap
  framing/token cleanup, bounded PTY ownership, and degraded fallback.
- The cross-language test verifies stable references across the in-process
  Clef/Tactus/Segno slice.

These tests do not validate a three-process authenticated deployment or the
missing agentrod database.

## Revisit Triggers

Revisit this ADR if cross-database atomicity cannot be achieved through current
outbox/idempotency contracts, external CAS cannot meet measured durability, a
platform cannot supervise external Python safely, or Electron removes the
required sandbox/preload controls. Do not revisit merely to avoid implementing
the missing transport or migration work.
