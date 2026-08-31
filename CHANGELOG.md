# Changelog

This project is a source alpha. Versions describe checked-in contracts; they
do not imply a package-registry release or compatibility guarantee.

## Unreleased

### Added

- Typed Clef norms, composable rubrics, honest checked/unchecked critiques,
  bounded guidance/refinement, and the open `agenstro.norm/v1` checker wire.
- A Python LaTeX norm checker with strict protocol and domain fixtures.
- Durable Tactus decision sessions with bounded `session list`, `session show`,
  and turn-CAS `session answer` commands plus append-only answer evidence.
- A Motivo Sessions view for findings, comparable choices, stakes, roadmaps,
  notes, multi-session selection, and stale-turn recovery.
- Reproducible local `Fast`, `Full`, `Release`, `Audit`, `Bootstrap`, and
  Cargo-clean quality profiles with machine-readable receipts and opt-in Git
  hooks.

### Changed

- Clef now gives ordinary plugins a finite one-hour transport deadline and
  layers a 13,500-second Tactus provider deadline below its four-hour outer
  provider supervisor.
- Published documentation carries checked ownership, applicability, platform,
  status, and verification metadata; the manual GitHub workflow remains a
  low-frequency cross-platform compiler matrix.

### Fixed

- Provider adapters retain at most 512 KiB of terminal result text while still
  draining bounded native output, and keep the safe 64 MiB generic process
  stdout default separate from provider-specific limits.
- Norm checks reject malformed bounds, loci, degenerate consistency groups,
  oversized sources/patterns, and non-terminating Python regex evaluation
  without falsely reporting a pass.
- Session listing uses bounded top-k retention while validating the full
  candidate set, and Motivo serializes controls/actions through child-process
  shutdown so commands cannot overtake one another.

### Compatibility and limitations

- Existing workspaces without `.tactus/sessions` remain valid and list no
  sessions; a later `tactus init` creates the additive directory.
- Planner registration, `session advance`, unattended defaults, transcript
  projection/mining, SARIF export, and a coordinated 0.4 version bump remain
  staged decisions rather than placeholder APIs.

## 0.3.0 — Haskell DSL, Rust runtime, and persistent tasks

### Added

- Haskell `clef-sdk` with typed `Workflow`, `Task`, `Operation`, and arbitrary
  `Plugin input output` composition.
- Incremental plugin events through a bounded `EventSink`, separate from typed
  terminal values.
- Rust Tactus workspace initialization, Haskell checking/running, provider
  adapters, process-tree supervision, bounded JSONL transport, and local run
  journals.
- TypeScript/React Motivo Studio as a redacted visual projection over the
  versioned Tactus control API.
- A per-user Windows x64 Motivo Studio installation and
  `motivo-studio [WORKSPACE]` command, with quoted-path handling and
  single-instance workspace switching.
- Haskell Segno persistent tasks with typed triggers and business state,
  durable lifecycle/cursors, interval and UTC-cron planning, SQLite CAS state,
  and at-least-once execution through Tactus.
- A model-free Windows active-window task and a deterministic multi-step
  topology workflow.

### Changed

- Project and package metadata now use GNU AGPL v3.0 only
  (`AGPL-3.0-only`).
- `tactus check` and `tactus run` accept repeatable `--package` extensions;
  Clef remains included automatically.
- Provider, effect, and generic plugin configuration uses the open
  `agenstro.plugin/v1` one-shot JSONL boundary.
- Long Segno task phases have a typed `--task-timeout-seconds` budget, and the
  Running lease is derived so it cannot expire before that budget.
- Repository-level fixtures and publication checks now live under `Test/`;
  Cargo, Cabal, MkDocs, and Electron Forge place rebuildable output below
  ignored `Build/`.

### Removed

- The superseded Clef and Tactus Rust product cores.
- The Python Tactus shim and its worker/daemon/cell execution model.
- The old Python/Rust Segno package, scheduler daemon, ZIP task format, and
  standalone Segno frontend.
- Motivo's legacy daemon/gRPC/PTY ownership; Motivo no longer acts as a second
  runtime.
- Frozen source archives and the unused 0.2 foundation crates/Protobuf tree;
  their history remains available in Git.
- Historical Python Clef case studies that no longer run against the current
  Haskell SDK; their complete sources remain available in Git history.

### Compatibility and limitations

- Existing 0.2 `.tactus` state is not migrated automatically. Initialize a
  clean 0.3 workspace or preserve the old directory separately.
- Workflows and plugins are trusted local code. There is no authentication or
  approval sandbox in this alpha.
- Segno is single-node and at-least-once. It does not provide arbitrary
  workflow replay, cross-backend exactly-once, distributed scheduling, or an
  automatic resolver for `OutcomeUnknown`.
- Motivo packages are unsigned and platform-local.

See the [migration guide](docs/migrations/0.2-to-haskell-0.3.md) and
[support matrix](docs/reference/support-matrix.md) for the current boundary.

## 0.2.x — historical foundation

The 0.2 line used Rust daemon/product cores, Python runtime shims, gRPC
boundaries, and a separate Segno scheduler model. It remains available in Git
history only and is not part of the 0.3 support gate.
