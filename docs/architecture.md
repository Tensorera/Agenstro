---
title: Agenstro 0.3 architecture
status: alpha
owners: [architecture]
last_verified: 2026-09-05
applies_to: "Clef Haskell 0.3.0.0, Tactus Rust 0.3.0, Segno Haskell 0.3.0.0, and Motivo Studio 0.3.0"
platforms: [windows, ubuntu]
---

# Agenstro 0.3 architecture

Agenstro separates task method, typed composition, execution, and scheduling.
Clef is a compact typed Haskell EDSL. Tactus is the Rust process/runtime kernel
that prepares a project, executes Clef programs, supervises plugins, routes
events, and records factual run evidence. Segno is the Haskell driver for typed
persistent tasks. Motivo Studio owns the replaceable method for a concrete
engineering task, its local reports and user notes, and the desktop interface.
It uses Tactus for provider execution and workspace projections.

The architectural split is:

| Component | Owns | Deliberately does not own |
| --- | --- | --- |
| Clef `0.3.0.0` | `Workflow a`, typed tasks/effects/plugins, explicit parallelism, typed requirements, typed norms/rubrics, incremental event sink, typed `Trigger state event`/`State state`/`PersistentTask` boundary | Provider catalogue, permission policy, custom language parser, scheduler loop, lifecycle database, artifacts, authentication |
| Tactus `0.3.0` | `.tactus` workspace, typed TOML, script selection, Cabal/GHC commands, one-shot dispatch, process groups, event journals, built-in adapters, Studio/session control DTOs and the durable session store | Haskell workflow/planner semantics, provider credentials, daemon/API service, replay, rollback, GUI |
| Segno `0.3.0.0` | Single-node driver, trigger cursors, occurrence lifecycle, leases/fences, SQLite state plugin, interval/cron planning, invocation/result handoff to Tactus | Workflow value semantics, distributed consensus, exactly-once effects, rollback, replay, provider execution |
| Motivo `0.3.0` | Replaceable task method, bounded lead/investigator calls, `.motivo` task records, Electron/preload boundary, React task views and existing session answers | Process-supervision kernel, Tactus config/trace/session ownership, Segno state, arbitrary renderer filesystem/shell access, daemon, scheduler, replay, credentials |
| Plugins | Provider/effect/domain behavior behind `agenstro.plugin/v1` | Core workflow composition and runtime ownership |

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

tactus run --all
    |
    +-> Cabal/GHC/runghc -> Clef Workflow a
                              |
                              | typed call / invoke / perform
                              v
                       tactus dispatch (one shot)
                              |
                              +-> same supervisor/journal path above

segno driver (Haskell, long lived)
        |
        +-> plan trigger leaves through Tactus -> interval / cron plugin
        +-> persist cursor + lifecycle --------> private lifecycle.sqlite3
        +-> load/CAS typed business state -----> segno.state plugin
                                                   -> business.sqlite3
        +-> tactus run --package segno-flow ---> one Clef PersistentTask
                                                    |
                                                    +-> Ignore / Complete /
                                                        Retry / Fail

Motivo renderer (sandboxed React)
        |
        | named, Zod-validated IPC
        v
Electron main (workspace root + task method + .motivo records)
        |
        | argv array, shell=false
        v
tactus dispatch --namespace provider
       + studio inspect/events + session list/show/answer
                       + generate/check/run/smoke
```

Clef programs and Tactus invocations remain one-shot. Motivo can request a
bounded sequence of agent calls while the application is open. The optional
Segno driver owns persistent scheduling: it must wait even when no workflow
process exists. Each trigger,
state, workflow, provider, and effect invocation is still a separately
correlated process operation; Segno does not expose a network daemon API.

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

`Norm artifact` extends the same static boundary to domain conventions.
Serialisable `CheckSpec` values travel through ordinary generic plugins,
`Rubric artifact` composes compatible norms, and `Critique` keeps checked and
unchecked identities separate. Bounded refinement remains normal `Workflow`
composition; it does not create another runtime or turn observations into
results.

Provider and generic plugin events are decoded as complete lines arrive. Clef
stores the frames in runtime records and passes each event to an `EventSink`.
The sink is an observation/projection surface, not `Workflow (Stream a)`: only
the terminal result is decoded into the workflow's declared output type. Clef
places records on a bounded queue, serializes a custom sink on one worker, and
boundedly attempts to flush the final value/evidence before `runWorkflow`
returns. Queue saturation, a stalled sink, or a sink exception stops further
projection and records one internal warning; it does not replace the typed
workflow result or manufacture an unknown provider outcome. Runtime records
retain the authoritative terminal independently of that projection.

The Haskell layer intentionally leaves these values open:

- provider and plugin names;
- model identifiers and reasoning effort;
- provider variants, argv additions, and environment additions;
- plugin-specific option objects; and
- event subtype payloads.

This prevents the core type model from becoming an out-of-date provider enum.
Typed convenience wrappers can be added at stable plugin boundaries.

Clef also defines the small typed handoff used by Segno. `Trigger state event`
has plugin-provided leaves and Haskell `mapTrigger`, `filterTrigger`,
`mergeTrigger`, and state-aware `gate` composition. `State state` carries a
stable key, schema version/migration, initial value, backend, and explicit
compare-and-set behavior. `PersistentTask` binds both to a Clef workflow that
returns `Ignore`, `Complete`, `Retry`, or `Fail`. Clef describes and executes
one occurrence; it does not schedule or persist lifecycle state.

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

Generation guidance comes from the TOML `instructions` path. The optional
`runtime_instructions` path supplies the separate instruction prefix for Clef
provider calls; omission means no prefix. Authoring instructions are never
implicitly prepended to a business invocation.

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

For generated Clef runtime configuration, the native provider CLI receives a
13,440-second deadline by default. Tactus retains 60 seconds to reap that
process and deliver a terminal frame before its
`limits.provider_timeout_seconds` dispatch deadline (13,500 seconds). Clef's
provider transport boundary uses `limits.provider_outer_timeout_seconds`
(14,400 seconds), and the enclosing workflow script owns the final outer
deadline through `limits.script_timeout_seconds` (15,300 seconds). Explicit
timeouts are preserved and validated against their supervisor; effects and
general plugins use `limits.plugin_timeout_seconds` (3,600 seconds). Request,
frame, stdout, event, and stderr budgets in the same optional object govern
Clef's outer plugin-v1 supervisor.

Each Clef runtime owns a provider semaphore, with four permits by default and
an override in `limits.max_concurrent_provider_calls`. A permit surrounds only
one `provider:` process boundary. It is never held around an enclosing
`Workflow`, observer lifecycle, or generic plugin/effect call. This bounds
agent fan-out without making nested workflow composition wait on a permit it
already owns. `parallelAllBounded` separately bounds arbitrary workflow
branches while preserving traversal order and structured sibling cancellation.

When no authoritative terminal result is available, Clef augments the
`PluginOutcomeUnknown` cause with phase, accepted frame/progress counts, the
last event type, its event-acceptance `last_event_unix_ms`, an explicit
`external_effect_possible` flag, and safe reconciliation guidance. The
timestamp is captured when Clef records the event, not synthesized when a
later timeout is diagnosed. Clef never copies the last event body into that
summary; prompt and model-output content remain confined to existing raw
runtime records. Provider-supplied detail objects are likewise withheld from
the summary and represented only by `reported_details_withheld`.

Transport validity and observation delivery are separate. Invalid or oversized
protocol data can make the transport outcome unknown. By contrast, the
observer callback queue reserves the authoritative terminal path and sheds
low-priority event projections when its frame or queue budget is full.
`events_dropped` counts those losses; callback failure or a missed observer
flush becomes `observation_error`. Neither changes `InvocationKind` or the
validated terminal result.

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
arbitrary sink on its polling thread. When the observational queue is full,
Tactus drops low-priority progress instead of failing or terminating the
invocation, and retains the terminal in the protocol state machine. A callback
that stalls or fails is likewise diagnostic degradation rather than a provider
result. The hidden `dispatch` command flushes accepted frames back to Clef,
whose incremental parser enqueues them for its isolated `EventSink` worker;
Clef records sink degradation without changing the typed return value.

Native diagnostics travel on stderr. They remain outside protocol frames and
typed workflow return values and are not automatically classified as errors.
Agenstro-generated user-log entries use only `[state]`, `[info]`, `[warning]`,
or `[error]` plus bounded natural language. Direct compiler/workflow output is
separate process output rather than a classified runtime log; Motivo keeps it
collapsed with raw stderr, protocol payloads, stable codes, and counters as
technical detail.

## Segno persistent-task driver

Segno owns the time and state that must survive between Clef processes. Its
driver repeatedly:

1. loads each installed task manifest and durable trigger cursor;
2. asks a trigger plugin to plan occurrences at the current observed time;
3. inserts each occurrence using its deterministic idempotency key;
4. advances the source cursor only after durable insertion;
5. claims ready work with an attempt, lease, and fencing token;
6. loads the typed business-state snapshot;
7. invokes the installed Haskell script through Tactus; and
8. validates and commits the script's explicit decision.

The driver owns waiting. Built-in `time.interval` and `time.cron` plugins are
pure planners: configuration plus cursor plus current time produces due
occurrences and a next wake time. They never call `sleep`. Missed occurrences
can therefore be calculated after restart and tested against a virtual clock.
Cron is UTC-only in version one.

An occurrence carries trigger and occurrence identities, logical and observed
time, cursor, idempotency key, attempt, and a typed payload. Delivery is at
least once. A local transport failure cannot prove that an external effect did
not complete, so an ambiguous execution becomes `OutcomeUnknown` instead of
an automatic retry.

## Segno state and plugin boundary

Scheduler lifecycle and workflow business state are deliberately separate.
Segno alone updates `Dormant`, `Ready`, `Claimed`, `Running`, `Waiting`,
`Succeeded`, `Failed`, and `OutcomeUnknown`, together with occurrence,
attempt, lease, and fence metadata. A user's `State state` value cannot mutate
those records.

The workflow reads an immutable `StateHandle state`. An explicit checkpoint is
a short compare-and-set operation and returns a handle with a new opaque
revision. No database transaction remains open while a workflow or agent runs.
A committed checkpoint is not rolled back if the workflow later fails or a
plugin has already changed external state. A business-state CAS and the later
lifecycle transition are separate durable facts; version one does not claim a
crash-atomic commit across the two SQLite databases.

Trigger leaves and state backends use the same open one-shot plugin process
shape as other Agenstro plugins. Trigger plugins expose `describe`, `plan`,
`poll`, `acknowledge`, and `smoke`; state plugins expose `describe`, `load`,
`compare-and-set`, `append`, `history`, and `smoke`. SQLite is the first local
backend. A future PostgreSQL, Redis, queue, filesystem, or webhook plugin can be
implemented in any language without widening Clef's Haskell core.

Segno keeps its durable files below the selected workspace, separate from
Tactus run evidence:

```text
.tactus/
  scripts/
  runs/
  sessions/
  segno/
    jobs/
    state/
    triggers/
```

## Motivo task method and interface

Motivo preserves Electron's process split. The React renderer has no Node
integration; a context-isolated preload exposes one named operation per IPC
channel. Electron main owns the selected root, task method, and task store;
it launches the external `tactus` executable without a shell. It never gives
the renderer an arbitrary command or filesystem primitive.

The default method offers `investigate`, `try`, `integrate`, and `conclude`.
They describe useful actions rather than a fixed order. A small change can be
completed directly. A task need not become a Haskell script. The method can use
existing tests and domain plugins, or create a small project plugin and fixtures
when a missing observation actually blocks progress. Harness behavior and
success criteria remain project-owned; Tactus still executes registered calls.

Each continuation has a provider-call budget: four by default, at most twenty.
The lead may request up to three independent investigation branches. Those
calls count against the same budget, and Motivo reserves one call for lead
integration before starting branches. Investigators receive separate prompts
and are instructed not to edit. They share the working environment; this is
not filesystem isolation or an enforced read-only capability. Dependent edits
remain with the sequential lead.

Each call runs through `tactus dispatch --namespace provider`. A call launches
the native coding agent's complete episode, which may itself use many model
and tool calls. Motivo's budget counts episodes, not tokens or internal tool
actions. A later call receives a bounded handoff from recent task reports and
source references; it does not resume a native agent session or replay tools.

Motivo atomically saves `motivo.task/v1` documents under
`.motivo/tasks/<uuid>.json`. The store includes goal, constraints, provider,
user notes, call timing, and structured reports. These are distinct from
Tactus diagnostic journals, legacy decision sessions, and Segno business state.
`.motivo/METHOD.md` may replace the default method text. The fixed report
protocol remains required so the application can read outcomes consistently.

Task states are `ready`, `running`, `paused`, `needs_input`, `completed`,
`failed`, and `outcome_unknown`. A pause waits for the current action and active
investigations to finish. Budget exhaustion saves a handoff and pauses. A
process interruption or unusable post-execution report is not automatically
retried; continuing an unknown outcome requires a user note describing what
was reconciled. Saving reports does not serialize a live continuation.

`completed` records the agent's delivery claim. Report validation checks the
document's structure, not whether a test was actually run or a requirement was
met. This method does not train model weights or establish an improvement in
model capability; those claims would need separate task-level evaluations.
See [ADR-0007](adr/0007-motivo-task-method.md).

Rust-owned queries form the read and decision boundary:

- `tactus studio inspect` returns health, ordered relative script names,
  redacted registries, and compact recent run state;
- `tactus studio events` validates an opaque run id and returns a bounded event
  page plus terminal summary and `ok`/`partial`/`corrupt` integrity.
- `tactus session list/show` return bounded `agenstro.session/v1` views; and
- `tactus session answer` validates a turn token, axis, and option under a
  per-session lock before atomically updating workspace-owned state.

Studio queries use a `tactus.control/v1` envelope with `agenstro.studio/v1`
data; session commands use the same envelope with `agenstro.session/v1` data.
Commands, plugin options, prompt text, and absolute script paths do not cross
the bridge.
All 64-bit counters in these Tactus projections are decimal strings. Motivo
does not parse TOML, walk Tactus journal directories, or infer task completion
from open event kinds. Its separate task reports may contain business content;
they are not the redacted Studio projection.

Projected events may carry a Tactus-owned `presentation` containing one of
`state`, `info`, `warning`, or `error` plus natural-language text. Motivo shows
that projection directly and keeps structured data collapsed as technical
evidence. It does not manufacture severity from stdout versus stderr or from an
open event-kind string. A true lifecycle change is recorded as
`runtime.state_transition` with `state_before`, `trigger`, `guard`, and
`state_after`; ordinary progress does not claim a transition.

Those four bracketed labels are the complete user-log vocabulary. Motivo may
add its own bounded `[warning]` when its action-output projection budget is
exhausted, but it keeps draining the child and discards only later raw frames.
Projection pressure never kills Tactus or changes the action outcome; raw
stdout/stderr and legacy events remain collapsed technical details.

The existing Sessions view adds a narrow inbound value without widening renderer authority. A
brief teaches with findings and consequences before asking exactly one
question. Motivo shows both the necessary question floor and conditional
surface, returns one bounded choice, and refetches rather than retrying a stale
turn. It never constructs a brief, selects a default, or writes session files.
The session planner and `session advance` remain deferred under ADR-0006.
Motivo Tasks use their own method/report boundary and do not mutate or advance
those legacy session documents.

## `agenstro.trace/v1` journal

Each supervised plugin call attempts to create a unique
`.tactus/runs/<run-id>/` directory. Journal I/O is observational: a writer
failure is recorded as degradation and cannot replace an already-known plugin
terminal or invocation kind. The journal has two publication rules:

- `events.jsonl` receives monotonically sequenced, append-flushed accepted
  diagnostic events as they occur;
- `summary.json` is written to a temporary file and atomically renamed after
  the terminal outcome is known, with an independent degraded-writer attempt
  so event loss does not hide that outcome.

Generation adds controller/provider/discovery events around its nested plugin
calls. A trace envelope contains the trace API, run ID, sequence, timestamp,
kind, optional presentation, and structured data. It is intentionally distinct
from `agenstro.plugin/v1`, which is the live process protocol.

The journal is diagnostic evidence, not an artifact store, replay contract, or
workflow state. Before persistence, Tactus recursively replaces prompt,
raw/text/content, credential-like, options, environment, workspace, and path
fields with byte-count/SHA-256 summaries, bounds remaining strings and arrays,
summarizes terminal success values, and withholds native stderr. It still does
not capture arbitrary Haskell `IO`, and bounded errors or path metadata may be
sensitive. Local retention and deletion remain the workspace owner's
responsibility.

## Provider adapters

Tactus includes translations for three native agent CLIs:

| Provider key | Native mode | Reasoning extension |
| --- | --- | --- |
| `codex` | `codex exec --dangerously-bypass-approvals-and-sandbox --json ...` | open `effort` |
| `claude-code` | `claude -p --dangerously-skip-permissions --output-format stream-json ...` | open `effort` |
| `opencode` | `opencode run --auto --format json ...` plus inline `permission=allow` | open `variant` (with effort fallback) |

Each adapter consumes the native stream to derive its live terminal value, but
does not forward token-level provider JSON or free text as user/journal events.
It emits one bounded `provider.diagnostic` aggregate containing counts, byte
sizes, event-type fingerprints, truncation state, and hashes. Offline `smoke`
resolves the executable/version; live smoke sends a minimal request. Tests use
fake executables and do not authenticate.

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

- network daemon/API, service discovery, or persistent provider session;
- authentication, capability token, approval UI, or credential broker;
- artifact tracker, workspace transaction, or rollback;
- exactly-once provider/effect guarantee or automatic retry after an
  ambiguous external outcome;
- global static DAG for arbitrary Haskell control flow; or
- deterministic replay of arbitrary Haskell `IO`.

Segno adds versioned business-state CAS and explicit checkpoints for persistent
tasks. It does not turn a workspace, provider invocation, or arbitrary `IO`
block into a transaction.

## Scheduling is not replay

Segno schedules a new occurrence and executes it through Tactus. It does not
substitute a recorded plugin result, serialize a Haskell continuation, or
intercept arbitrary `IO`. Re-running an occurrence can perform external work
again. Exactly-once delivery, distributed multi-node scheduling, automatic
external-effect rollback, and arbitrary workflow replay remain explicit
non-goals for this release.

See the [plugin protocol](reference/plugin-protocol-v1.md), [support
matrix](reference/support-matrix.md), [Segno guide](segno.md), and
[ADR-0004](adr/0004-haskell-segno-persistent-tasks.md) for the exact current
boundary.
