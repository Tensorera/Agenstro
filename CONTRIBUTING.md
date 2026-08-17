# Contributing to Agenstro

Agenstro `0.3` has four current implementation surfaces: Clef is the Haskell
EDSL, Tactus is the Rust execution kernel, Segno is the Haskell persistent-task
driver, and Motivo Studio is the TypeScript/React visual projection. Do not
revive the removed Python/Rust Segno stack or a legacy daemon path.

All contributions are accepted under the repository's GNU AGPL v3.0-only
license. Do not copy code whose license is incompatible with `AGPL-3.0-only`;
retain required third-party notices and record new dependencies explicitly.

## Ownership boundaries

| Path | Ownership |
| --- | --- |
| `clef-sdk/haskell/src/` | Current Clef EDSL and runtime boundary |
| `clef-sdk/haskell/test/` | Haskell workflow/protocol contract tests |
| `clef-sdk/haskell/examples/` | Small compilable Clef examples |
| `tactus-runtime/src/*.rs` | Current Rust CLI, typed config, process supervisor, adapters, and journal |
| `tactus-runtime/tests-rust/` | Current Tactus and adapter tests |
| `examples/topology-holes/` | Current four-stage integration example |
| `docs/` | Current documentation plus clearly marked migration history |
| `motivo-studio/` | Current Electron main/preload/React projection over Tactus control DTOs |
| `segno-flow/haskell/src/` | Current Segno driver, lifecycle, plugin hosts, and SQLite backend |
| `segno-flow/haskell/test/` | Offline virtual-clock, persistence, and protocol tests |
| `segno-flow/examples/` | Explicit opt-in persistent-task examples; default gates use fakes |
| `segno-flow/segno-flow.cabal` | Current Haskell package and `segno` executable |
| `Test/` | Repository-level fixtures and publication contract tests |
| `Build/` | Ignored generated output; tools recreate their own subdirectories |

Old Segno Python/Rust packages and their independent desktop surface are not
current compatibility targets. Git history is the migration record.

## Design rules

1. Keep Clef small. Prefer ordinary `do` notation, typed inputs/outputs, and an
   explicit `parallel` boundary over a global DAG or type-level effect row.
2. Keep provider policy out of `Workflow`. Models, effort, variants, permission
   flags, credentials, and provider-specific options belong to configuration
   and adapters.
3. Keep Tactus typed and deterministic at its boundary. Parse TOML into
   category-specific Rust structures, reject invalid configuration early, and
   preserve open nested plugin options.
4. Preserve the language-neutral `agenstro.plugin/v1` process contract.
   Commands are argv arrays; stdout is UTF-8 protocol JSONL; stderr is
   diagnostics.
5. Route complete event frames incrementally. Events and diagnostics are
   evidence, not values silently inserted into a workflow's typed result.
6. Treat every configured plugin and Haskell program as trusted local code.
   Argument arrays and process groups improve correctness; they are not an
   authentication or sandbox boundary.
7. Keep `agenstro.trace/v1` factual and append-only. Do not describe a run
   journal as replay, exactly-once execution, an artifact store, or rollback.
8. Keep `workspace.paths` observational. It may report a final path delta, but
   it does not own files, retain contents, detect reads, or prove attribution.
9. Keep persistent scheduling in Segno. Its versioned business-state CAS and
   explicit checkpoints must remain separate from Tactus journals and from
   Segno-owned lifecycle records.
10. Keep Motivo a projection. Renderer code cannot import Node/Electron, and
    Electron main must call versioned Tactus control commands instead of
    parsing `tactus.toml`, `runtime.json`, or trace directories.

## Documentation ownership

Each public fact has one canonical page:

- installation and upgrades: `docs/install.md`;
- the first controlled tutorial: `docs/getting-started.md`;
- Clef authoring: `docs/clef.md`;
- workspace/config schema: `docs/tactus-workspace.md`;
- provider configuration: `docs/providers.md`;
- plugin implementation: `docs/plugin-authoring.md`;
- logs and state transitions: `docs/observability.md`;
- backup/retention/recovery: `docs/operations.md`; and
- exact commands/wire shapes: `docs/reference/`.

README files summarize and link to those pages; they must not become a second
copy of the reference contract.

## Haskell changes

The Cabal package uses GHC2021 and the `base` bounds in
`clef-sdk/clef-sdk.cabal`.

- Put public modules under `clef-sdk/haskell/src/Clef/` and re-export only the
  intended compact surface from `Clef.hs`.
- Prefer typed wrappers around the open JSON plugin boundary. Keep `rawPlugin`
  available as the explicit escape hatch.
- Keep plugin event sinks orthogonal to `Workflow a`; a progress event is not a
  workflow result.
- Do not hide asynchronous exceptions or cancellation inside broad handlers.
- Add focused tests for malformed frames, incremental delivery, correlation,
  terminal-result rules, typed decode failures, and generic plugins.
- Examples used by the gate must not contact a real provider.

Run from the repository root:

```powershell
cabal build --builddir=Build/cabal all --enable-tests
cabal test --builddir=Build/cabal all --test-show-details=direct
```

## Rust Tactus changes

Keep the runtime separated into the existing concerns:

- `workspace`: idempotent layout, typed TOML, script discovery, runtime JSON;
- `process`: one-shot process-group supervision, deadlines, cancellation,
  bounded pipes, incremental frame validation;
- `journal`: flushed JSONL events and atomic terminal summaries;
- `adapters`: Codex, Claude Code, OpenCode, and `workspace.paths` translations;
- `cli`: the public command surface and error/exit-code mapping.

The public CLI contract is `init`, `list`, `prompt`, `generate`, `check`,
`run`, `doctor`, `smoke`, and `plugin-call`. `dispatch`, `provider-host`, and
`effect-host` are internal subcommands used to keep all plugin calls under the
same Rust supervisor.

Add regression coverage for:

- strict TOML/JSON validation and open plugin options;
- malformed, oversized, Unicode, duplicate, missing, and post-terminal frames;
- immediate event delivery and bounded backpressure;
- deadline/cancellation cleanup of descendants on Windows and Unix;
- journal ordering and atomic summary publication;
- fake provider argv/output translation, including model/effort/variant; and
- `workspace.paths` exclusions and delta semantics.

Real provider calls are opt-in manual tests and must never run in the default
gate.

## Haskell Segno changes

Keep the public model smaller than the scheduler implementation:

- trigger leaves belong to open plugins; Haskell owns only typed
  `mapTrigger`, `filterTrigger`, `mergeTrigger`, and state-aware `gate`;
- do not add an `and` combinator without explicit correlation-key and time-
  window semantics;
- keep business `State state` physically and logically separate from lifecycle,
  attempts, leases, fencing tokens, and trigger cursors;
- use short compare-and-set transactions; never hold a database transaction
  over a workflow or provider call;
- keep interval/cron planning pure and let the driver own waiting;
- execute every installed Clef task through Tactus rather than bypassing its
  process/plugin boundary; and
- preserve the honest at-least-once claim. Ambiguous external outcomes become
  `OutcomeUnknown`, not an automatic unsafe retry.

The built-in active-window example and all default Segno tests must run without
a model. Use a fake window source and virtual time for deterministic scheduling
coverage; a real foreground-window smoke is explicit and platform-local.

Version one does not implement exactly-once execution, distributed drivers,
serialized Haskell continuations, arbitrary workflow replay, or rollback of
external effects.

### Keep generated output centralized

The checked-in Cargo configuration writes to `Build/cargo`; Cabal, MkDocs, and
Electron Forge use sibling directories under the same ignored `Build/` root.
Clean Cargo's configured directory in `finally` after a full validation run:

```powershell
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo fmt --all --check
  cargo check -p tactus-runtime --all-targets --locked
  cargo test -p tactus-runtime --locked
  cargo clippy -p tactus-runtime --all-targets --locked -- -D warnings
} finally {
  cargo clean
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}
```

Use package-scoped checks while iterating. Run broader workspace checks only
when the change actually crosses package boundaries.

For Motivo Studio changes, exercise every TypeScript boundary and the packaged
Electron asset graph:

```powershell
npm --prefix motivo-studio ci
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio run typecheck
npm --prefix motivo-studio test
npm --prefix motivo-studio run package
```

These tests use a fake Tactus process. Never put provider credentials into the
desktop test or packaging environment.

## Changing the plugin or trace protocol

The canonical plugin contract is
[`docs/reference/plugin-protocol-v1.md`](docs/reference/plugin-protocol-v1.md).
A wire change is incomplete unless these layers agree:

- Clef's encoder, incremental parser, event sink, and error mapping;
- Tactus' strict decoder, supervisor, dispatcher, and journal projection;
- built-in provider/effect adapters;
- cross-language and malformed-input tests; and
- the protocol and support documentation.

Keep plugin event payloads open unless interoperability genuinely requires a
closed schema. Every one-shot call must produce exactly one terminal result for
the matching request ID. Human logs never belong on protocol stdout.

`agenstro.trace/v1` is a separate local journal envelope. Changing it does not
implicitly change `agenstro.plugin/v1`, and neither version promises replay.

## Documentation

The MkDocs site publishes the current Clef/Segno Haskell, Tactus Rust, and
Motivo Studio path plus selected migration material. Historical pages may
remain excluded, but must not be presented as current support.

- Update `docs/getting-started.md` when installation or CLI syntax changes.
- Update `docs/architecture.md` when runtime ownership changes.
- Update protocol/reference pages for field-level changes.
- Update `docs/troubleshooting.md` for reproducible user-facing failures.
- Update `docs/roadmap.md` when a surface or release gate changes.

Validate navigation and links:

```powershell
python -m mkdocs build --strict
```

Python in that command belongs to MkDocs only; it is not a Tactus runtime
requirement.

## Sensitive data and generated output

Never commit:

- `.tactus/` state copied from a target project, especially run journals;
- provider credentials, `.env` files, private keys, or machine-local registry
  configuration;
- `secretdoc/` private design notes;
- `Build/`, package-local `node_modules/`/`.vite/`, Python caches, generated
  output, or model transcripts; or
- target-project artifacts that are not deliberate test fixtures.

Before committing, inspect the staged scope:

```powershell
git status --short
git diff --cached --stat
git diff --cached --check
```

## Pull requests

Keep a change scoped to one architectural concern. State:

- the user-visible outcome;
- which current or historical surfaces changed;
- exact validation commands and results;
- whether any live provider request occurred; and
- remaining limitations.

Passing tests are evidence for the exercised behavior, not a claim that
arbitrary generated workflows are safe, deterministic, or replayable.
