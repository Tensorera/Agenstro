# Contributing to Agenstro

Agenstro `0.3` has three current implementation surfaces: Clef is the Haskell
EDSL, Tactus is the Rust execution kernel, and Motivo Studio is its
TypeScript/React visual projection. Segno Flow stays frozen. Do not introduce
another workflow runtime or silently revive a legacy daemon path.

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
| `archive/` | Read-only historical snapshots |
| `crates/`, `proto/` | Retained legacy foundation code; not the Tactus `0.3` kernel |
| `motivo-studio/` | Current Electron main/preload/React projection over Tactus control DTOs |
| `segno-flow/` | Frozen; no current feature or packaging claim |

Changes to Segno should be limited to build hygiene, migration documentation,
or a separately approved revival plan.

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
9. Do not add a daemon, credential broker, CAS, checkpoint system, or implicit
   retry policy to the `0.3` core without a separate design decision.
10. Keep Motivo a projection. Renderer code cannot import Node/Electron, and
    Electron main must call versioned Tactus control commands instead of
    parsing `tactus.toml`, `runtime.json`, or trace directories.

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
cabal build all --enable-tests
cabal test all --test-show-details=direct
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

### Keep Cargo artifacts out of the checkout

Do not use the repository's default `target/` for validation. Allocate a unique
system-temporary target and clean it in `finally`:

```powershell
$targetDir = Join-Path $env:TEMP ("agenstro-target-" + [guid]::NewGuid().ToString("N"))
$env:CARGO_TARGET_DIR = $targetDir
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo fmt --all --check
  cargo check -p tactus-runtime --all-targets --locked
  cargo test -p tactus-runtime --locked
  cargo clippy -p tactus-runtime --all-targets --locked -- -D warnings
} finally {
  cargo clean --target-dir $targetDir
  Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
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

The MkDocs site publishes only the current Clef Haskell/Tactus Rust path plus
selected migration material. Historical pages may remain excluded, but must not
be presented as current support.

- Update `docs/getting-started.md` when installation or CLI syntax changes.
- Update `docs/architecture.md` when runtime ownership changes.
- Update protocol/reference pages for field-level changes.
- Update `docs/troubleshooting.md` for reproducible user-facing failures.
- Update `docs/roadmap.md` when a frozen surface or release gate changes.

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
- `target/`, `dist-newstyle/`, `node_modules/`, Electron `out/`/`.vite/`,
  caches, generated site output, or model transcripts; or
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
- which current or frozen surfaces changed;
- exact validation commands and results;
- whether any live provider request occurred; and
- remaining limitations.

Passing tests are evidence for the exercised behavior, not a claim that
arbitrary generated workflows are safe, deterministic, or replayable.
