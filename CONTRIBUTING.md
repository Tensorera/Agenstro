# Contributing to Agenstro

Agenstro `0.3` keeps the user's existing coding agent as its first interface.
Clef is the Haskell EDSL, Tactus is the Rust execution kernel, Segno is the
Haskell persistent-task driver, and Motivo supplies a project skill, eight
Haskell method templates, and offline HTML observation. Do not revive the
removed desktop task loop, Python/Rust Segno stack, or legacy daemon path. New daemon supervision is an opt-in Tactus component;
the Haskell Segno service owns persistent scheduling.

All contributions are accepted under the repository's GNU AGPL v3.0-only
license. Do not copy code whose license is incompatible with `AGPL-3.0-only`;
retain required third-party notices and record new dependencies explicitly.

PowerShell 7 is the repository-level task runner on both supported platforms.
The canonical local gates are:

```powershell
./scripts/quality.ps1 -Profile Fast
./scripts/quality.ps1 -Profile Full
```

The commands never contact a live model provider. Run
`./scripts/quality.ps1 -Profile Bootstrap` explicitly when a checkout needs
network-fetched tool or package dependencies.

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
| `skills/motivo/` | Method choice and evidence guidance for the existing coding agent |
| `skills/tactus/` | Workflow authoring, daemon supervision and live-output guidance |
| `skills/segno/` | Persistent organizer authoring, service recovery and offline templates |
| `motivo/templates/` | Eight ordinary Haskell methods and shared record/report helpers |
| `motivo/tests/` | Offline method and installed-asset acceptance tests |
| `motivo-studio/` | Self-contained HTML template, reading controls, and static build/tests |
| `plugins/motivo-test/` | Rust experiment effect with Linux isolation and a bounded shared deadline |
| `segno-flow/haskell/src/` | Current Segno driver, lifecycle, plugin hosts, and SQLite backend |
| `segno-flow/haskell/test/` | Offline virtual-clock, persistence, and protocol tests |
| `segno-flow/examples/` | Explicit opt-in persistent-task examples; default gates use fakes |
| `segno-flow/segno-flow.cabal` | Current Haskell package and `segno` executable |
| `Test/` | Repository-level fixtures and publication contract tests |
| `Build/` | Ignored generated output; tools recreate their own subdirectories |
| `scripts/` and `.githooks/` | Canonical local quality entrypoint and opt-in Git hooks |

Old Segno Python/Rust packages and their independent desktop surface are not
current compatibility targets. Git history is the migration record.

## Design rules

1. Keep Clef small. Prefer ordinary `do` notation, typed inputs/outputs, and an
   explicit `parallel` boundary over a global DAG or type-level effect row.
2. Keep provider policy out of `Workflow`. Models, effort, variants, permission
   flags, credentials, and provider-specific options belong to configuration
   and adapters. The shared `InvocationPermissions` request is provider-neutral;
   adapters and capability negotiation own application and fallback warnings.
   Record requested and effective permissions separately. A call override
   replaces the startup request as a whole without editing user configuration.
3. Keep Tactus typed and deterministic at its boundary. Parse TOML into
   category-specific Rust structures, reject invalid configuration early, and
   preserve open nested plugin options.
4. Preserve the language-neutral `agenstro.plugin/v1` process contract.
   Commands are argv arrays; stdout is UTF-8 protocol JSONL; stderr is
   diagnostics.
5. Route complete event frames incrementally. Events and diagnostics are
   evidence, not values silently inserted into a workflow's typed result.
6. Treat configured plugins and Haskell programs as trusted local code.
   Argument arrays and process groups are not an authentication or universal
   sandbox boundary. The separate `motivo.test` plugin enforces a specific
   experiment boundary; do not extend that claim to arbitrary workflow IO.
7. Keep `agenstro.trace/v1` factual and append-only. Do not describe a run
   journal as replay, exactly-once execution, an artifact store, or rollback.
8. Keep `workspace.paths` observational. It may report a final path delta, but
   it does not own files, retain contents, detect reads, or prove attribution.
9. Keep persistent scheduling in Segno. Its versioned business-state CAS and
   explicit checkpoints must remain separate from Tactus journals and from
   Segno-owned lifecycle records.
10. Keep the main conversation and method selection in the existing coding
    agent. Motivo skill guidance and Haskell templates assist it; Tactus owns
    execution. Method sources belong in `.tactus/motivoscript`, business
    sources in `.tactus/scripts`, and method evidence in `.tactus/motivo`.
11. Make methods independently selectable. Prefer existing notes and an
    existing template; an independent provider call must select the configured
    provider explicitly. Report publication and agent claims do not prove task
    correctness. HTML is a read-only snapshot, with no task controller,
    provider selection, execution button, runtime Node/Electron, or port.
12. Run Motivo experiments through `motivo.test`. Its supported Linux backend
    confines persistent writes to `.tactus/motivotest` and shares one deadline
    of at most 600 seconds across preparation and execution. Unsupported or
    unavailable isolation must refuse execution, not fall back to the host.
    Do not automatically repeat interrupted or ambiguous actions.

## Documentation ownership

Each public fact has one canonical page:

- installation and upgrades: `docs/install.md`;
- the first controlled tutorial: `docs/getting-started.md`;
- Clef authoring: `docs/clef.md`;
- workspace/config schema: `docs/tactus-workspace.md`;
- provider configuration: `docs/providers.md`;
- plugin implementation: `docs/plugin-authoring.md`;
- logs and state transitions: `docs/observability.md`;
- agent-led methods and offline reports: `docs/motivo-studio.md`;
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
./scripts/quality.ps1 -Profile Fast
```

## Rust Tactus changes

Keep the runtime separated into the existing concerns:

- `workspace`: idempotent layout, typed TOML, script discovery, runtime JSON;
- `process`: one-shot process-group supervision, deadlines, cancellation,
  bounded pipes, incremental frame validation;
- `journal`: flushed JSONL events and atomic terminal summaries;
- `adapters`: Codex, Claude Code, OpenCode, Antigravity, and `workspace.paths` translations;
- `protocol`: strict incremental `agenstro.plugin/v1` frame decoding;
- `studio`: the versioned read-only Studio control projection;
- `session`: validated durable decision-session storage and control commands;
- `daemon`: bounded provider supervision, repair proposals and escalation;
- `watch`: ephemeral local provider streams and the read-only browser viewer;
- `skills`: bundled guidance, installation baselines and conservative skill updates;
- `cli`: the public command surface and error/exit-code mapping.

The public CLI contract is `init`, `skills update`, `list`, `prompt`, `generate`, `check`,
`run` (including opt-in `--daemon`), `watch`, `doctor`, `smoke`, `plugin-call`,
`runs`, `studio`, and `session`. `dispatch`, `provider-host`, and
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

The repository gate must include the separate Rust experiment plugin, not only
the Tactus package:

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

`motivo-test` isolation acceptance uses real Bubblewrap on Linux and requires
usable user/PID/mount namespaces. Other platforms must retain the structured
unsupported result; never replace isolation with a permissive test fallback.

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
Motivo's static build use sibling directories under the same ignored `Build/` root.
Every quality profile finishes by enforcing a project cache budget below
**2 GiB (2,147,483,648 bytes, binary)**, including after failed checks. Compilation can
temporarily exceed that budget. Project-local Node/Python tool dependencies
are retained and counted; Cargo/Cabal output and generated sites are removed
when necessary. Logs and summaries remain available.

```powershell
./scripts/quality.ps1 -Profile Full
./scripts/quality.ps1 -Profile Clean
./scripts/build-cache.ps1 -MeasureOnly
```

Available main executables for the current host platform and Cabal source
distributions are saved under
`Build/artifacts/cargo/{debug,release}/`, `Build/artifacts/segno/`, and
`Build/artifacts/cabal/` before compiler output is removed. Those deliverables
are outside the cache budget. Global Cargo/Cabal caches and installed toolchains
are not cleaned. See [the quality commands](scripts/README.md) for the standalone
cleanup command and a smaller budget.

Cargo dev/test profiles share `debug = 0` and `incremental = false`, reducing
debugger detail and disabling reuse within a changed crate. Unchanged dependency
builds are still reused until cleanup. Direct build commands do not enforce the
budget; run `./scripts/build-cache.ps1` after them when needed.

Use package-scoped checks while iterating. Run broader workspace checks only
when the change actually crosses package boundaries.

For Motivo Studio changes, exercise the standalone HTML reader and static build:

```powershell
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio test
npm --prefix motivo-studio run build
```

`npm ci` belongs to the explicit Bootstrap or clean Release path rather than
every warm-cache iteration.

Reader tests cover offline reading, escaped content, and reading controls
without a task API or adjacent-file fetch. Keep provider credentials out of
all fixtures. Core content must remain readable without JavaScript; timestamps
describe a saved snapshot, not a live connection.

Rebuild Tactus after changing embedded skills, Haskell templates, or the HTML
asset. The installed-only method test exercises exactly what initialization
ships, rather than silently copying newer repository files into the workspace:

```sh
cargo build --workspace --locked
python3 motivo/tests/run.py --tactus Build/cargo/debug/tactus --installed-only
python3 motivo/tests/tdd.py --tactus Build/cargo/debug/tactus --motivo-test Build/cargo/debug/motivo-test --installed-only
```

Keep the embedded skill file list aligned with all three skills. Installation
is confined to `.tactus/skills` for explicit use; preserve existing host trees
and legacy mirrors without creating new discovery files. Skill-upgrade tests must preserve customized and unrecognized
files, including across repeated initialization, and keep dry-run free of writes.
The Segno service acceptance executes the organizer and business assets installed
by Tactus, so missing or stale packaged templates fail the end-to-end check.

On Windows use the available Python command and `Build/cargo/debug/tactus.exe`.
For template-only iteration, omitting `--installed-only` lets the test runner
copy current source assets into its isolated test workspace. Validate input
preservation, method-specific artifacts, source/run references, interrupted
experiments, identity provenance, and actual Haskell execution. The default
tests do not contact a real model. [ADR-0008](docs/adr/0008-agent-led-motivo.md)
records the method and observation boundary.

`Motivo.Test` suites check exit codes, protected logs and sample artifacts from
`motivo.test`. Red needs an actual assertion failure with complete evidence;
green must pass the same normalized command/assertion manifest. Infrastructure
failures, incomplete logs and provider claims cannot establish either phase.
The Full gate checks those distinctions and a real Linux sandbox red/green
cycle using isolated fixtures. Unsupported platforms type-check the templates
and report skipped execution coverage; they never run experiments on the host.

The reference norm checker is a standalone Python plugin. Its fixture runner
checks both domain results and the JSONL terminal/correlation rules:

```powershell
python plugins/latex-norm-check/run_fixtures.py
```

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

`Fast` checks the metadata schema and local link targets. `Full` additionally
renders the strict MkDocs navigation:

```powershell
./scripts/quality.ps1 -Profile Full
```

Python in these repository checks belongs to MkDocs, offline Motivo acceptance,
and the optional reference norm plugin; it is not a Tactus or Motivo runtime
requirement.

## Sensitive data and generated output

Never commit:

- `.tactus/` state copied from a target project, especially run journals;
- `.tactus/motivo/` reports or `.tactus/motivotest/` experiments copied from a
  target project, including embedded business evidence; old `.motivo/tasks/`
  histories remain sensitive too;
- provider credentials, `.env` files, private keys, or machine-local registry
  configuration;
- `secretdoc/` private design notes;
- `Build/`, package-local `node_modules/`, Python caches, generated
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

## Commit messages

Use Conventional Commits for new work: `feat(scope):`, `fix(scope):`,
`docs(scope):`, `test(scope):`, `refactor(scope):`, `ci(scope):`, or
`chore(scope):`. Keep the subject imperative and name the architectural
surface when one component owns the change.

The optional tracked hooks call the same repository gates. Install them for
this checkout with:

```powershell
./scripts/install-hooks.ps1
```

The pre-push hook runs `Full` with the same default 2 GiB project cache budget.
Global Cargo/Cabal caches and toolchains are left alone. It requires a clean
worktree and accepts only the current `HEAD` as a non-deletion push source;
pre-commit rejects unstaged or untracked source so the files tested are the
files being committed.
