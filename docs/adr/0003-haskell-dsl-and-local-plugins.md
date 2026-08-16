# ADR-0003: Haskell DSL and trusted local plugins

- Status: Accepted
- Date: 2026-08-15
- Scope: Clef 0.3 refactor
- Supersedes: the Clef and Tactus ownership decisions in ADR-0001 and ADR-0002

## Context

The 0.2 implementation puts workflow modelling, scheduling, persistence,
artifact publication, worker supervision, and provider adaptation behind a
Rust/Python daemon stack. That implementation is well tested, but it is the
wrong centre of gravity for the next design: workflows must be ordinary
Haskell programs written by coding agents, while provider- and environment-
specific behaviour must remain replaceable plugins.

The runtime is explicitly for trusted local development in this phase. It does
not claim that an LLM invocation, an arbitrary Haskell program, or a locally
configured plugin is a security boundary.

## Decision

### 1. The DSL is real Haskell

Clef is a Haskell library. GHC is the parser and type checker; Clef will not
implement a second, merely Haskell-like language. A workflow script may import
and use any Haskell module available to its Cabal project.

The first core is intentionally small:

```haskell
Workflow a
Task input output
Operation output

invoke     :: Task input output -> input -> Workflow output
invokeWith :: ProviderRef -> Task input output -> input -> Workflow output
perform    :: Operation output -> Workflow output
parallel   :: Workflow a -> Workflow b -> Workflow (a, b)
require    :: (a -> Bool) -> a -> Workflow a
```

`Task input output` makes provider data flow type checked. `Operation output`
does the same for an effect call. `invoke` and `perform` stay separate because
they answer different questions:

- a provider answers *who performs an agent task*;
- an effect answers *how the program observes or changes the outside world*.

### 2. Do not encode a false global guarantee

The first version does not use `Workflow effects pre post a`, a custom indexed
`do`, or a whole-program DAG compiler. Ordinary Haskell branches can depend on
provider results, plugins are discovered at runtime, and scripts may use `IO`
directly. A type-level effect row would therefore describe only cooperative
DSL calls while looking like a complete authority boundary.

The runtime records the dependency and lifecycle that actually happen.
`parallel` is the only Clef primitive that requests concurrency. More precise
typed wrappers may be supplied by individual plugins without enlarging the
core model.

### 3. Plugins use one small cross-language ABI

Provider and effect implementations are ordinary local executables. One
invocation starts one process, writes one JSON request to stdin, accepts JSONL
events on stdout, and requires exactly one terminal result. Diagnostics belong
on stderr.

```json
{"api":"agenstro.plugin/v1","id":"...","method":"invoke","params":{}}
{"type":"event","id":"...","event":{}}
{"type":"result","id":"...","ok":true,"value":{}}
```

The wire format keeps provider options as JSON values. Model names, reasoning
effort, variants, and future provider-specific switches are not closed Haskell
enums. A plugin may add a typed Haskell convenience module while retaining the
open JSON escape hatch.

There is no plugin authentication, daemon token, permission broker, or
sandbox policy in this phase. Commands and credentials are inherited from the
user environment. The bundled coding-agent adapters deliberately select each
provider's least-interactive mode. This is trusted local configuration and is
documented as arbitrary code execution.

### 4. Tactus is the typed Rust execution kernel

Tactus owns the project-local `.tactus` convention, typed plugin configuration,
script discovery, compiler invocation, ordered command-line execution,
bounded one-shot process supervision, streaming event routing, cancellation,
and factual `agenstro.trace/v1` journals. It does not own cells, notebooks,
artifacts, CAS, rollback, a database, a daemon, or an approval state machine.

Generated entry scripts use `.tactus/scripts/NNN_slug.hs` (or `.lhs`) so a
coding agent can create several ordered workflows. Naming is a prompt and
listing convention, not a language restriction: helper modules and explicitly
selected files may use any valid Haskell filename and content. `check` delegates
to Cabal/GHC, and `run` launches each script as a normal CLI program.

### 5. The first effect observes workspace paths

`workspace.paths` snapshots metadata and content digests, then reports added,
modified, deleted, and type-changed paths around a provider invocation. It is
observation only: it stores no artifact content, performs no rollback, and
does not decide whether a mutation is allowed.

Snapshot differences cannot observe reads or transient writes, and concurrent
writers make attribution ambiguous. Cooperative operations can later emit
more precise access events without changing that limitation.

### 6. Motivo projects Tactus; Segno remains frozen

The cutover initially froze Motivo. The `0.3` follow-up revives only a thin
TypeScript/React projection over versioned, redacted Rust control queries.
Electron main owns the selected root and Tactus child; the sandboxed renderer
receives named, schema-checked IPC. Motivo does not parse runtime state or own a
second daemon/workflow runtime.

Segno Flow's current cron/lease scheduler is not a replay engine. It is frozen
until the plugin trace format is stable. A future replay feature must state
whether it reuses recorded plugin results or explicitly performs live calls;
arbitrary Haskell `IO` is not generally replayable.

## Migration

The Haskell package and Rust Tactus `0.3` path are authoritative. Superseded
Clef/Tactus product cores remain available through Git history; selected Python
archives remain migration evidence and must not gain new features. Motivo's
projection is current; Segno remains frozen until its future role is redesigned
explicitly.

## Consequences

- GHC reports task and effect value-type mistakes before a script runs.
- Core code no longer needs to know Codex, Claude Code, OpenCode, filesystem
  hashing, or future effect domains.
- Providers and effects can be implemented in Haskell, Rust, Python, Node, or
  any language that can speak the JSONL ABI.
- Direct `IO` and direct shell commands can bypass cooperative effects. This is
  an explicit capability of ordinary Haskell scripts, not an accidental safety
  claim.
- Dynamic control flow is executed and traced rather than forced into a static
  DAG that cannot faithfully represent it.
