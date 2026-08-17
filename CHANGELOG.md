# Changelog

This project is a private source alpha. Versions describe checked-in contracts;
they do not imply a public package release or compatibility guarantee.

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

- `tactus check` and `tactus run` accept repeatable `--package` extensions;
  Clef remains included automatically.
- Provider, effect, and generic plugin configuration uses the open
  `agenstro.plugin/v1` one-shot JSONL boundary.
- Long Segno task phases have a typed `--task-timeout-seconds` budget, and the
  Running lease is derived so it cannot expire before that budget.
- Repository-level cases, fixtures, and publication checks now live under
  `Test/`; Cargo, Cabal, MkDocs, and Electron Forge place rebuildable output
  below ignored `Build/`.

### Removed

- The superseded Clef and Tactus Rust product cores.
- The Python Tactus shim and its worker/daemon/cell execution model.
- The old Python/Rust Segno package, scheduler daemon, ZIP task format, and
  standalone Segno frontend.
- Motivo's legacy daemon/gRPC/PTY ownership; Motivo no longer acts as a second
  runtime.
- Frozen source archives and the unused 0.2 foundation crates/Protobuf tree;
  their history remains available in Git.

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
