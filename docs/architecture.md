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
persistent tasks. The user's existing coding agent owns the conversation and
chooses Motivo methods. Motivo supplies a project skill, eight editable Haskell
templates, and evidence reports; Motivo Studio renders read-only HTML snapshots.
There is no second agent client or application-owned task loop.

The architectural split is:

| Component | Owns | Deliberately does not own |
| --- | --- | --- |
| Clef `0.3.0.0` | `Workflow a`, typed tasks/effects/plugins, explicit parallelism, typed requirements, typed norms/rubrics, incremental event sink, typed `Trigger state event`/`State state`/`PersistentTask` boundary | Provider catalogue, permission policy, custom language parser, scheduler loop, lifecycle database, artifacts, authentication |
| Tactus `0.3.0` | `.tactus` workspace, typed TOML, script selection, Cabal/GHC commands, one-shot dispatch, process groups, event journals, built-in adapters, Studio/session control DTOs and the durable session store | Haskell workflow/planner semantics, provider credentials, daemon/API service, replay, rollback, GUI |
| Segno `0.3.0.0` | Single-node driver, trigger cursors, occurrence lifecycle, leases/fences, SQLite state plugin, interval/cron planning, invocation/result handoff to Tactus | Workflow value semantics, distributed consensus, exactly-once effects, rollback, replay, provider execution |
| Motivo `0.3.0` | Skill guidance, eight Haskell method templates, `.tactus/motivo` records and artifacts, offline HTML observation | Main conversation, provider registry, process-supervision kernel, Tactus sessions, Segno state, autonomous task loop, replay, credentials |
| `motivo.test` effect | Linux experiment isolation and a shared deadline of at most 600 seconds | General Haskell sandboxing, business-workflow policy, remote experiments, automatic retries |
| Plugins | Provider/effect/domain behavior behind `agenstro.plugin/v1` | Core workflow composition and runtime ownership |

## Component flow

```text
user -> existing coding agent -> project Motivo / Tactus skills
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

existing coding agent
        +-> select a method and prepare evidence
        +-> tactus run --scripts-dir .tactus/motivoscript --script ...
                         |
                         +-> ordinary Clef/Haskell method
                              +-> .tactus/motivo records and artifacts
                              +-> optional motivo.test experiment
                                   -> .tactus/motivotest/<run>/<sample>
                         |
                         +-> self-contained HTML snapshot -> user browser
        <--- method evidence informs the next business script or investigation
```

Clef programs, Motivo templates, and Tactus invocations remain one-shot. The
existing coding agent decides whether another method or business action is
useful; an open HTML page never schedules work. The optional
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
  motivoscript/
  motivotest/
  motivo/
    index.html
    runs/
  skills/
    tactus/
    motivo/
  runs/
  sessions/
```

Initialization also installs short project skill discovery entries for Codex,
Claude Code, and OpenCode. They point to the canonical `.tactus/skills` files;
existing project instructions and edited templates are preserved. `list`,
`check`, and `run` accept a generic `--scripts-dir` below `.tactus`. Discovery,
explicit source validation, and Haskell module lookup use that selected tree.
The default `.tactus/scripts` tree never includes Motivo methods in `--all`.
Source selection is not an IO sandbox.

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
separate process output rather than a classified runtime log. Method reports
can retain experiment output as technical evidence without assigning severity
from stdout or stderr alone.

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

## Motivo methods and offline observation

The user initializes a folder, starts a chosen coding agent there, and asks it
to use the Motivo skill. The main agent selects among clarify, investigate,
analyze, research, probe, organize, retrospect, and handoff. These are
independent methods, not mandatory stages. The default templates publish notes
prepared in that existing conversation; they do not silently start another
provider. An explicit independent provider call uses the existing Tactus
registry and Clef composition. Requested model identity, caller-reported
identity, and identity confirmed by execution are distinct facts.

Method entries and helpers live under `.tactus/motivoscript`, apart from
business entries under `.tactus/scripts`. The method guidance lives in the
project skill and ordinary editable Haskell; Clef has no method-specific type
system and Tactus has no method-selection loop. Project tests and plugins
supply observations. A useful small plugin can be developed for a specific
observation gap without imposing a universal task-correctness contract.

Each method publishes input, ordered samples, method-specific artifacts, a
short local reflection, and a human-readable report under
`.tactus/motivo/runs/<run-id>`. Reports preserve the distinction between
observations, interpretations, and unresolved questions. Related runs can
reference earlier runs; timestamps alone do not establish causality or a
required cross-method order. A record marked `recorded` means notes were
published, not that the user's engineering goal was achieved. An interrupted
experiment can leave partial evidence and requires inspection before repetition.

The `probe` method uses the separate `motivo.test` effect. Its Linux backend
uses Bubblewrap filesystem and PID namespaces: project material is read-only,
and persistent experiment writes are confined to
`.tactus/motivotest/<run>/<sample>`. The default and maximum experiment deadline
are 600 seconds; smaller positive values are accepted. One monotonic deadline
covers preparation and experiment execution. Missing isolation support and
unsupported platforms refuse experiment execution. This plugin's constraint
does not change the privileges of arbitrary Haskell, providers, or other
plugins, and does not make a business operation transactional.

Motivo Studio is a self-contained HTML projection of those records. Inline
styles and pre-rendered core content work offline, without Node, Electron,
IPC, a server, or a listening port at runtime. Small browser controls support
reading; no page action selects a provider or executes work. Each publication
replaces a complete snapshot. Refreshing loads a newer snapshot, and the page
states when it was generated. It neither subscribes to a live agent nor
claims an old `running` record proves that a process is still alive.

Old `.motivo/tasks` documents remain historical. They are not automatically
deleted, replayed, resumed, or converted into the new evidence model.
[ADR-0008](adr/0008-agent-led-motivo.md) supersedes the desktop-owned
method and task loop. Methods still require evaluation against direct-agent
work; a report format is not evidence of improved model capability.

## Tactus control projections remain independent

Existing Rust-owned control APIs remain available to command-line tools and
other consumers:

- `tactus studio inspect` returns health, ordered relative business-script
  names, redacted registries, and compact recent run state;
- `tactus studio events` validates an opaque run id and returns a bounded event
  page plus terminal summary and `ok`/`partial`/`corrupt` integrity;
- `tactus session list/show` return bounded `agenstro.session/v1` views; and
- `tactus session answer` validates a turn token, axis, and option under a
  per-session lock before atomically updating workspace-owned state.

Studio queries use a `tactus.control/v1` envelope with `agenstro.studio/v1`
data; session commands use the same envelope with `agenstro.session/v1` data.
Commands, plugin options, prompt text, and absolute script paths are withheld
from these projections. Their 64-bit counters are decimal strings. Motivo's
method artifacts contain business content and are separate from these redacted
Tactus APIs; deleting the old client does not remove the APIs or transfer
ownership of sessions.

Projected events may carry Tactus-owned `presentation` values using `state`,
`info`, `warning`, or `error`. A lifecycle change is recorded as
`runtime.state_transition` with `state_before`, `trigger`, `guard`, and
`state_after`; ordinary progress does not claim a transition. The session
planner and `session advance` remain deferred under ADR-0006.

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
