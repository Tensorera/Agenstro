# Tactus Runtime

Tactus `0.3.0` is the Rust execution kernel and project-local CLI for Clef
Haskell workflows. It owns workspace initialization, typed configuration,
script discovery, Cabal/GHC process execution, one-shot plugin supervision,
incremental event routing, and local run journals.

The canonical user references are [Installation](../docs/install.md),
[Tactus workspace and configuration](../docs/tactus-workspace.md), and the
[CLI reference](../docs/reference/cli-v0.3.md). This component README stays
focused on the Rust package and implementation boundary.

It intentionally does not contain a daemon, authentication system, credential
broker, artifact/CAS service, checkpoint restore, or rollback engine.

Tactus is licensed with the rest of Agenstro under
[GNU AGPL v3.0 only](../LICENSE).

## Install

From the Agenstro repository root:

```powershell
cargo install --path tactus-runtime --bin tactus --locked --force
tactus --version
```

Tactus itself has no Python runtime dependency. You also need Cabal/GHC to
check or run Clef scripts. A native `codex`, `claude`, or `opencode` executable
is needed only when using that provider adapter.

## Initialize a workspace

Run this from the project the workflow will modify:

```powershell
tactus init --sdk D:\src\Agenstro\clef-sdk
```

If Tactus can find the SDK from the checkout layout, `--sdk` may be omitted.
Initialization creates only missing content:

```text
.tactus/
  tactus.toml       typed provider/effect/plugin configuration
  cabal.project     local Clef package linkage
  PROMPT.md         generation instructions
  scripts/          ordinary Haskell entries and helper modules
  runs/             per-invocation trace directories
```

Runnable entries are named `NNN_slug.hs` or `NNN_slug.lhs`. Tactus orders them
by the three-digit prefix and then relative path. Other Haskell files are
helpers: `check` sees them, but `run` does not select them implicitly.

## CLI

```powershell
tactus init
tactus list
tactus prompt
tactus generate --provider codex "create a typed multi-step workflow"
tactus check
tactus check .tactus\scripts\010_contract.hs
tactus run
tactus run --script .tactus\scripts\010_contract.hs -- --workflow-argument
tactus doctor
tactus smoke
tactus smoke provider:codex --live
tactus plugin-call workspace.paths describe --namespace effect
tactus studio inspect
```

- `init` creates missing project-local control files without replacing existing
  ones.
- `list` classifies ordered entries and helpers.
- `prompt` prints the resolved generation instructions.
- `generate` invokes the selected provider and asks it to write numbered DSL
  sources; it does not check or run those sources.
- `check` builds Clef and invokes GHC with `-fno-code`.
- `run` executes selected entries through `runghc`; `--script` is repeatable.
- `doctor` validates layout, configuration, SDK linkage, tools, and configured
  plugin commands.
- `smoke` calls each selected plugin's offline `smoke` method unless `--live`
  is explicit. Prefix ambiguous names with `provider:`, `effect:`, or
  `plugin:`.
- `plugin-call` calls any registered method with an open JSON object.
- `studio` exposes the bounded, redacted control projection consumed by Motivo;
  it is not a second daemon or workflow API.

`check` and `run` are fail-fast unless `--keep-going` is supplied. Supervised
commands have a default 1,800-second deadline; use `--timeout-seconds 0` on a
command that exposes the option only when deliberately disabling it.

## Typed configuration

`.tactus/tactus.toml` is decoded into separate provider, effect, and generic
plugin definitions. Tactus rejects unknown category-specific fields, empty
commands, an unregistered `default_provider`, non-finite numbers, and TOML
values that cannot cross the JSON boundary. Nested `options` remain open.

The initialized configuration uses built-in Tactus subcommands:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus", "provider-host", "codex"]
effort = "high"

[providers."claude-code"]
command = ["tactus", "provider-host", "claude-code"]

[providers.opencode]
command = ["tactus", "provider-host", "opencode"]

[effects."workspace.paths"]
command = ["tactus", "effect-host", "workspace-paths"]
observe_invocations = true

[plugins]
```

`provider-host`, `effect-host`, and `dispatch` are internal subcommands of the
single `tactus` binary. Generated runtime JSON points Clef calls at `dispatch`,
which resolves the original configured argv and keeps every call under the Rust
supervisor.

## Process and event model

Each plugin invocation is one process:

1. Tactus starts its argv directly in the workspace, without shell parsing.
2. It writes one UTF-8 `agenstro.plugin/v1` request and closes stdin.
3. It reads and validates complete JSONL frames incrementally.
4. It forwards each event immediately and records it in order.
5. It requires exactly one correlated terminal result and then reaps the group.

The supervisor uses a Unix process group or Windows Job Object, has bounded
request/frame/stdout/stderr/event-queue limits, isolates potentially blocking
event sinks from its polling loop, and terminates owned descendants on deadline,
cancellation, protocol failure, queue overflow, or sink stall. Windows Job
Objects contain the nested tree; a Unix process that deliberately creates a new
session can escape process-group containment, so plugins remain trusted local
code. A valid process outcome says what the local process did; it cannot prove
whether a remote provider completed work before a connection failed.

Each call receives a directory below `.tactus/runs/`:

```text
.tactus/runs/<run-id>/
  events.jsonl      flushed agenstro.trace/v1 factual events
  summary.json      atomically published terminal process outcome
  runtime.json      resolved Clef/plugin dispatch configuration when needed
```

The trace is local diagnostic evidence. It may contain provider output and is
not a replay format, authorization record, artifact manifest, or rollback log.

## Motivo Studio control projection

Motivo does not parse `tactus.toml`, `runtime.json`, or trace files. It uses two
read-only, versioned control queries owned by this Rust binary:

```powershell
tactus studio inspect --root D:\work\project --run-limit 50
tactus studio events run-... --root D:\work\project --after 0 --limit 250
```

Both commands emit exactly one `tactus.control/v1` JSON envelope. The projected
`agenstro.studio/v1` data contains ordered relative script names, redacted
registry metadata, generic health status, compact recent runs, and bounded trace
pages. It never exposes configured command arrays, plugin options, prompt text,
or absolute script paths. Sequence numbers, timestamps, and event counts are
decimal strings so TypeScript clients do not lose 64-bit precision.

`studio events` accepts only a validated opaque run id, does not follow symlink
trace entries, limits each line to 1 MiB and each request to at most 8 MiB, and
reports `ok`, `partial`, or `corrupt` integrity. Unknown event kinds remain open
JSON data: Studio may display them but must not reinterpret them as replayable
workflow state.

## Built-in adapters

The Rust binary includes:

- Codex: non-interactive `codex exec` with
  `--dangerously-bypass-approvals-and-sandbox`;
- Claude Code: print/stream mode with `--dangerously-skip-permissions`;
- OpenCode: `opencode run --auto --format json` with an inline
  `permission=allow` configuration; and
- `workspace.paths`: snapshot/diff/forget plus observer begin/end operations.

Models, effort, OpenCode variants, extra argv, and environment additions are
adapter options rather than Rust/Haskell enums. Offline smoke probes resolve
the executable and version. Live smoke sends a minimal real request and can be
billable.

OpenCode's permission mode is weaker than the explicit Codex and Claude Code
bypass flags: explicit deny and managed configuration can still take
precedence. Tactus reports this caveat instead of claiming full equivalence.

`workspace.paths` compares path type, size, and SHA-256 without retaining file
content. It excludes `.git`, effect-internal state/run data, and common build
trees (`target`, `node_modules`, `build`, and `dist-newstyle`). A snapshot is
bounded to 100,000 paths, 512 MiB hashed, and 30 seconds. It cannot observe
reads, transient writes, or reliably attribute concurrent changes; it never
restores files. `observe.end` is idempotent for its opaque token: a durable
metadata-only completion record lets a retry recover the same delta and is
eligible for bounded cleanup after 24 hours.

## Arbitrary-language plugins

Provider/effect registries are convenience categories, not a closed plugin
universe. Add any one-shot executable under `[plugins.<name>]` and call it from
Clef with `jsonPlugin`/`rawPlugin` or from the CLI with `plugin-call`. Rust,
Haskell, TypeScript, C#, Python, and shell-independent native tools are all
valid implementations if stdout contains only protocol JSONL and diagnostics
go to stderr.

See the repository's [plugin protocol](../docs/reference/plugin-protocol-v1.md)
for the complete frame lifecycle.

## Development without a large local `target`

```powershell
$targetDir = Join-Path $env:TEMP ("agenstro-tactus-target-" + [guid]::NewGuid().ToString("N"))
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

The tests use fake provider executables. They do not contact a model service.
For the end-to-end multi-step example, see
[`../examples/topology-holes`](../examples/topology-holes/).
