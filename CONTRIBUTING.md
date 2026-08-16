# Contributing to Agenstro

Agenstro is in a Haskell-first `0.3` cutover. Before changing code, decide
whether the work belongs to the current release surface or to a frozen legacy
surface. Do not quietly revive a second workflow runtime in Rust, Python, or
the UI.

## Current ownership boundaries

| Path | Ownership |
| --- | --- |
| `clef-sdk/haskell/src/` | Current Clef EDSL and runtime boundary |
| `clef-sdk/haskell/test/` | Current Haskell contract tests |
| `clef-sdk/haskell/examples/` | Compilable workflow examples |
| `tactus-runtime/src/tactus_runtime/` | Current Python 3.12 CLI and reference plugin hosts |
| `tactus-runtime/tests/` | Current Tactus and plugin tests |
| `docs/` | Current docs plus explicitly marked migration history |
| `archive/` | Read-only `0.2` source snapshots |
| `crates/`, `proto/` | Frozen Rust/protobuf foundation |
| `motivo-studio/`, `segno-flow/` | Frozen during the `0.3` cutover |

Changes to a frozen surface should be limited to build hygiene, migration
documentation, or a separately approved revival plan. Current feature claims
must be backed by the Haskell/Tactus release gate.

## Design rules

1. Keep the Haskell core small. Prefer normal `do` notation, typed inputs and
   outputs, and explicit `parallel` composition over a global DAG, scheduler,
   or type-level effect row.
2. Keep provider-specific policy out of `Workflow`. Model identifiers,
   reasoning effort, permission flags, credentials, and future provider options
   belong to adapters and runtime configuration.
3. Treat plugins as trusted local executables. Do not introduce a partial
   authentication or sandbox story and describe it as a security boundary.
4. Preserve the language-neutral `agenstro.plugin/v1` boundary. Commands are
   argv arrays, not shell strings; stdout is protocol JSONL and stderr is
   diagnostics.
5. Make failure semantics explicit. A provider transport loss after work may
   have occurred is `outcome_unknown`; do not imply rollback or safe retry.
6. Keep path observation observational. It may report final workspace deltas,
   but it does not own files, restore contents, or prove attribution.

## Haskell changes

The Cabal package uses GHC2021 and supports the `base` bounds declared in
`clef-sdk/clef-sdk.cabal`. Avoid compiler-specific extensions unless the
supported range and CI matrix are updated intentionally.

- Put public modules under `clef-sdk/haskell/src/Clef/` and re-export the small
  intended surface from `Clef.hs`.
- Prefer concrete typed wrappers around the open JSON plugin boundary.
- Do not hide asynchronous exceptions or cancellation inside broad exception
  handlers.
- Add a focused test to `clef-sdk/haskell/test/Main.hs` for every wire or
  workflow contract change.
- Keep examples compilable; examples must not contact a real provider during
  the test gate.

Run from the repository root:

```powershell
cabal build all --enable-tests
cabal test all --test-show-details=direct
```

## Tactus and plugin changes

Tactus targets Python `3.12` with strict Pyright and Ruff checks.

- Put runtime code under `tactus-runtime/src/tactus_runtime/`.
- Keep CLI imports side-effect free.
- Use `pathlib.Path`, explicit encodings, typed dataclasses, and structured
  domain errors at public boundaries.
- Preserve caller cwd/environment/stdin/stdout behavior for Haskell scripts.
- Keep `init` idempotent and never overwrite unknown `.tactus` content.
- Add protocol regression tests for malformed frames, Unicode, correlation,
  duplicate/missing terminal results, non-finite numbers, process failure, and
  `outcome_unknown` behavior.
- Unit tests must use fake provider executables. Real model calls are opt-in
  manual smoke tests and must never enter default CI.

Set up and run the gate:

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -e ".\tactus-runtime[dev]"

python -m pytest tactus-runtime/tests
python -m ruff check tactus-runtime/src/tactus_runtime tactus-runtime/tests
python -m ruff format --check tactus-runtime/src/tactus_runtime tactus-runtime/tests
python -m pyright -p tactus-runtime/pyproject.toml
```

Run `ruff format` only on files within your change. Do not mechanically rewrite
frozen or user-owned work that is unrelated to the task.

## Changing the plugin protocol

The canonical contract is
[`docs/reference/plugin-protocol-v1.md`](docs/reference/plugin-protocol-v1.md).
A protocol change is incomplete unless all affected layers agree:

- the Haskell encoder/parser and runtime error mapping;
- the Python client and reference host writer/reader;
- provider and effect adapters;
- golden or cross-language tests; and
- the protocol reference and migration notes.

Keep event payload schemas open unless interoperability requires a closed
field. A process must produce exactly one terminal result for the matching
request ID. Do not write human diagnostics to protocol stdout.

## Documentation

The MkDocs site contains only the current Haskell/Tactus path plus selected
migration material. Historical pages may remain in Git while excluded from the
site, but they must not be presented as current support.

- Update `docs/getting-started.md` when installation or CLI steps change.
- Update `docs/architecture.md` when ownership or runtime boundaries change.
- Update protocol/reference pages for field-level contract changes.
- Update `docs/troubleshooting.md` for a reproducible user-facing failure.
- Update `docs/roadmap.md` when a frozen surface or release gate changes.

Validate links and navigation:

```powershell
python -m pip install "mkdocs>=1.6,<2"
python -m mkdocs build --strict
```

Examples should state their working directory, distinguish PowerShell from
POSIX shells, and say whether a command can make a billable provider request.

## Sensitive data and generated output

Never commit:

- `.tactus/` runtime state from a target project;
- provider credentials, `.env` files, private keys, or machine-local package
  registry configuration;
- `secretdoc/` private design notes;
- build output, caches, generated site output, or model transcripts; or
- target-project artifacts that are not deliberate test fixtures.

Before committing, inspect both the file list and staged diff:

```powershell
git status --short
git diff --cached --stat
git diff --cached --check
```

If a test requires a binary fixture, document its provenance and purpose. Do
not add a public license or redistribute third-party material without explicit
authorization.

## Pull requests

Keep a change scoped to one architectural concern. The description should
state:

- the user-visible outcome;
- which current or frozen surfaces changed;
- the exact validation commands and results;
- whether any live provider request was made; and
- remaining limitations or follow-up work.

Passing tests are evidence for the behavior they exercise, not a general claim
that arbitrary generated workflows are safe, deterministic, or replayable.
