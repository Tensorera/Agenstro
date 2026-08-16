---
title: Agenstro 0.3 architecture
status: alpha
last_verified: 2026-08-15
applies_to: "Clef Haskell 0.3.0.0 and Tactus Rust 0.3.0"
---

# Agenstro 0.3 architecture

Agenstro has two authoritative runtime components and one visual projection.
Clef is a compact typed Haskell EDSL. Tactus is the Rust process/runtime kernel
that prepares a project, executes Clef programs, supervises plugins, routes
events, and records factual run evidence. Motivo Studio is a TypeScript/React
desktop client that asks Tactus for redacted, versioned projections.

The architectural split is:

| Component | Owns | Deliberately does not own |
| --- | --- | --- |
| Clef `0.3.0.0` | `Workflow a`, typed tasks/effects/plugins, explicit parallelism, typed requirements, incremental event sink | Provider catalogue, permission policy, custom language parser, daemon, global scheduler, artifacts, authentication |
| Tactus `0.3.0` | `.tactus` workspace, typed TOML, script selection, Cabal/GHC commands, one-shot dispatch, process groups, event journals, built-in adapters, Studio control DTOs | Haskell workflow semantics, provider credentials, daemon/API service, CAS, replay, rollback, GUI |
| Motivo `0.3.0` | Electron window/preload boundary, React visualization, named Zod IPC, one top-level Tactus action | Workflow semantics, config/trace parsing, general filesystem/shell access, daemon, scheduler, replay, credentials |
| Plugins | Provider/effect/domain behavior behind `agenstro.plugin/v1` | Core workflow composition and runtime ownership |

Segno Flow remains frozen code. It is outside the `0.3` execution graph and
release gate.

## Component flow

```text
user / coding agent
        |
        | tactus init/list/prompt/generate/check/run/doctor/smoke/plugin-call
        v
+--------------------------- Tactus (Rust) ----------------------------+
| typed workspace config -> CLI orchestration -> process supervisor     |
|                                     |              |                  |
|                                     |              +-> trace journal |
|                                     v                                 |
|                         one-shot plugin dispatch                       |
+-----------------------------|-----------------------------------------+
                              | agenstro.plugin/v1 JSONL
                 +------------+-------------+----------------+
                 v                          v                v
          provider adapter            effect adapter    generic plugin
       Codex / Claude / OpenCode      workspace.paths     any language
                 |
                 v
          native agent CLI

tactus run
    |
    +-> Cabal/GHC/runghc -> Clef Workflow a
                              |
                              | typed call / invoke / perform
                              v
                       tactus dispatch (one shot)
                              |
                              +-> same supervisor/journal path above

Motivo renderer (sandboxed React)
        |
        | named, Zod-validated IPC
        v
Electron main (workspace root + child handle)
        |
        | argv array, shell=false
        v
tactus studio inspect/events + generate/check/run/smoke
```

There is no resident service between these boxes. Each invocation is
self-contained and correlated by request/run identifiers.

## Clef: static composition, open runtime

`Workflow a` is an abstract wrapper around `Runtime -> IO a`. It uses ordinary
Haskell control flow; GHC checks the types passed between steps.

The main building blocks are:

- `Task input output` for provider-shaped work;
- `Operation output` for configured effects;
- `Plugin input output` for any other registered one-shot process;
- `jsonPlugin` for `ToJSON`/`FromJSON` boundaries and `rawPlugin` for an
  explicit `Value` escape hatch;
- `parallel` for explicit structured concurrency;
- `require`/`requireBecause` for typed guards; and
- `attempt` for catching workflow-domain failures without swallowing
  asynchronous cancellation.

Provider and generic plugin events are decoded as complete lines arrive. Clef
stores the frames in runtime records and passes each event to an `EventSink`.
The sink is an observation/projection surface, not `Workflow (Stream a)`: only
the terminal result is decoded into the workflow's declared output type. Clef
places records on a bounded queue, serializes a custom sink on one worker, and
boundedly flushes the final value/evidence before `runWorkflow` returns. A sink
failure is distinct from a provider whose external outcome is unknown.

The Haskell layer intentionally leaves these values open:

- provider and plugin names;
- model identifiers and reasoning effort;
- provider variants, argv additions, and environment additions;
- plugin-specific option objects; and
- event subtype payloads.

This prevents the core type model from becoming an out-of-date provider enum.
Typed convenience wrappers can be added at stable plugin boundaries.

## Tactus workspace and typed configuration

`tactus init` creates only missing paths:

```text
.tactus/
  tactus.toml
  cabal.project
  PROMPT.md
  scripts/
  runs/
```

`tactus.toml` has three registries:

- `[providers.<name>]`: command plus optional model, effort, and open options;
- `[effects.<name>]`: command, open options, and observer participation; and
- `[plugins.<name>]`: command plus open options for arbitrary domain plugins.

Rust structures distinguish those categories and reject unknown fields in each
definition. Cross-field validation requires `api = "clef.runtime/v1"`, a
registered default provider, and non-empty argv. Options must fit the JSON data
domain; TOML datetimes and non-finite floats are rejected. Nested option names
and string values such as model/effort remain deliberately open.

Before running Haskell, Tactus materializes runtime JSON and sets
`TACTUS_RUNTIME_CONFIG`. Plugin commands in that document point at the exact
current `tactus dispatch` executable, not directly at the configured plugin.
This gives Clef a language-neutral configuration while retaining Rust process
supervision for every call.

## One-shot dispatch and process groups

For one plugin request Tactus:

1. resolves one exact provider/effect/plugin registry entry;
2. starts its argv in the workspace without a shell;
3. places it in a Unix process group or Windows Job Object;
4. writes one bounded UTF-8 request while concurrently draining stdout and
   stderr;
5. validates JSONL frames incrementally with a bounded queue;
6. forwards each accepted event immediately;
7. requires one correlated terminal result and a coherent process exit; and
8. reaps the owned process group on completion, deadline, cancellation, or
   protocol failure.

The supervisor bounds request size, frame size, aggregate stdout, retained
stderr, frame count, and pending-event count. CLI commands expose wall-clock
deadlines; `0` deliberately disables a deadline where supported. These bounds
limit local resource use. Windows Job Objects contain the nested process tree;
on Unix, a process that deliberately creates a new session can escape
process-group containment. Plugins are therefore still trusted local code.
Local termination also cannot prove whether a remote provider completed an
operation before a transport failure, and never implies a safe retry.

`tactus check` and `tactus run` apply the same process-group ownership to
Cabal/GHC/runghc. Their terminal streams remain attached to the caller so
compiler and program output stays visible.

## Incremental event routing

The wire protocol has two record kinds:

```json
{"type":"event","id":"r1","event":{"type":"progress","message":"..."}}
{"type":"result","id":"r1","ok":true,"value":{}}
```

Tactus does not wait for process exit before handling events. A reader thread
separates LF-terminated frames, a strict decoder validates them, and a bounded
queue hands them to an isolated sink worker. The supervisor never calls an
arbitrary sink on its polling thread: queue overflow or a sink that misses its
delivery deadline fails the invocation and triggers process-group cleanup. The
hidden `dispatch` command flushes each validated frame back to Clef, whose
incremental parser enqueues it for its isolated `EventSink` worker.

Human diagnostics travel on stderr. They remain outside protocol frames and
typed workflow return values.

## Motivo Studio projection

Motivo preserves Electron's process split. The React renderer has no Node
integration; a context-isolated preload exposes one named operation per IPC
channel. Electron main owns the selected root and launches an external
`tactus` executable without a shell. It never gives the renderer an arbitrary
command or filesystem primitive.

Two Rust-owned queries form the read boundary:

- `tactus studio inspect` returns health, ordered relative script names,
  redacted registries, and compact recent run state;
- `tactus studio events` validates an opaque run id and returns a bounded event
  page plus terminal summary and `ok`/`partial`/`corrupt` integrity.

Both use a `tactus.control/v1` envelope with `agenstro.studio/v1` data. Commands,
plugin options, prompt text, and absolute script paths do not cross the bridge.
All 64-bit counters are decimal strings. Motivo therefore does not parse TOML,
walk the journal directory, infer workflow state from open event kinds, or own
a competing scheduler/replay model.

## `agenstro.trace/v1` journal

Each supervised plugin call creates a unique `.tactus/runs/<run-id>/`
directory. The journal has two publication rules:

- `events.jsonl` receives monotonically sequenced, append-flushed factual
  events as they occur;
- `summary.json` is written to a temporary file and atomically renamed after
  the terminal outcome is known.

Generation adds controller/provider/discovery events around its nested plugin
calls. A trace envelope contains the trace API, run ID, sequence, timestamp,
kind, and structured data. It is intentionally distinct from
`agenstro.plugin/v1`, which is the live process protocol.

The journal is not an artifact store or replay contract. It may contain
prompts/provider output, has no built-in redaction or credential service, and
does not capture arbitrary Haskell `IO`. Local retention and deletion remain
the workspace owner's responsibility.

## Provider adapters

Tactus includes translations for three native agent CLIs:

| Provider key | Native mode | Reasoning extension |
| --- | --- | --- |
| `codex` | `codex exec --dangerously-bypass-approvals-and-sandbox --json ...` | open `effort` |
| `claude-code` | `claude -p --dangerously-skip-permissions --output-format stream-json ...` | open `effort` |
| `opencode` | `opencode run --auto --format json ...` plus inline `permission=allow` | open `variant` (with effort fallback) |

Each adapter normalizes native streaming output into open plugin events and a
terminal value. Offline `smoke` resolves the executable/version; live smoke
sends a minimal request. Tests use fake executables and do not authenticate.

OpenCode has a deliberate capability caveat: `--auto` approves ask decisions,
but an explicit deny or managed policy may still win. Tactus reports
`full_bypass=false` rather than claiming parity with the explicit Codex and
Claude Code dangerous flags.

Provider login, credential storage, pricing, model availability, and
organization policy belong to the native CLI. Tactus adds none of its own.

## `workspace.paths` effect

The built-in effect implements `describe`, offline `smoke`, `snapshot`, `diff`,
`forget`, `observe.begin`, and `observe.end`. Configured as an observer, it
takes a path snapshot before a provider call and reports the final delta after
the call:

```json
{"added":[],"modified":[],"deleted":[],"type_changed":[]}
```

Snapshots compare path kind, file size, and SHA-256. Public evidence contains
workspace-relative paths, not file contents. The effect excludes `.git`, its
internal state/run data, and `target`, `node_modules`, `build`, and
`dist-newstyle`; it does not apply every `.gitignore` rule. One snapshot is
bounded to 100,000 paths, 512 MiB hashed, and 30 seconds.

Observer completion is an idempotent commit. A same-token retry reads the
durable completion value and cleans any residual pre-observation state; bounded
garbage collection makes completion records eligible for removal after 24
hours. This is crash recovery for evidence, not workflow replay.

It cannot observe reads, an intermediate file that disappears before the final
snapshot, or the identity of a concurrent writer. It does not authorize,
publish, restore, or roll back anything.

## Trust and non-goals

Haskell scripts, configured plugins, built-in provider CLIs, and arbitrary
`liftIO` all run with the user's ambient authority. `argv` execution avoids
shell-string ambiguity, protocol validation rejects malformed data, and
process groups make termination more reliable; none creates hostile-code
isolation.

The `0.3` architecture has no:

- daemon, socket API, service discovery, or persistent provider session;
- authentication, capability token, approval UI, or credential broker;
- artifact tracker, CAS, checkpoint, workspace transaction, or rollback;
- exactly-once provider/effect guarantee or automatic retry;
- global static DAG for arbitrary Haskell control flow; or
- deterministic replay of arbitrary Haskell `IO`.

## Frozen surface

Segno Flow remains a scheduling/replay exploration. Any revival must separate
recorded-result replay from a new live provider invocation and acknowledge that
ordinary Haskell `IO` is not interceptable. It does not participate in the
current build/runtime claim.

See the [plugin protocol](reference/plugin-protocol-v1.md), [support
matrix](reference/support-matrix.md), and [roadmap](roadmap.md) for the exact
current boundary.
