---
title: Segno persistent tasks
status: alpha
owners: [segno]
last_verified: 2026-08-17
applies_to: "Segno Flow Haskell 0.3.0.0"
platforms: [windows, ubuntu]
---

# Segno persistent tasks

Segno keeps a typed Clef task triggerable after its Haskell process exits. It
is a Haskell library and single-node driver, not the removed Python/Rust Segno
stack and not a replay engine.

The core relationship is:

```text
Trigger state event + State state + Clef Workflow decision
                         |
                         v
                    PersistentTask
```

Trigger and state implementations are open one-shot plugins. Haskell supplies
the typed wrappers and composition; Segno owns durable time, cursor, occurrence
and lifecycle state; Tactus executes every plugin call and every actual Clef
job.

## Install Segno

Use the canonical [installation guide](install.md) to install or upgrade
Tactus and the `segno` executable. This guide assumes both commands resolve and
that `tactus check --help` contains the repeatable `--package` option.

## Initialize a project

For a new project, initialize Tactus first and then Segno in that project:

```powershell
$projectRoot = (Resolve-Path D:\work\my-project).Path

tactus init $projectRoot --sdk (Join-Path $repoRoot "clef-sdk")
segno init --root $projectRoot --sdk (Join-Path $repoRoot "segno-flow")
```

For an existing `.tactus` workspace, verify its marker and add Segno's files
and registrations in place:

```powershell
$projectRoot = (Resolve-Path D:\work\my-existing-project).Path
if (-not (Test-Path (Join-Path $projectRoot ".tactus\tactus.toml"))) {
  throw "This folder is not initialized; run tactus init first"
}

tactus doctor --root $projectRoot
segno init --root $projectRoot --sdk (Join-Path $repoRoot "segno-flow")
```

`segno init` is idempotent. It adds the built-in trigger/state/domain plugin
registrations to `.tactus/tactus.toml`, links `segno-flow` from
`.tactus/cabal.project`, and creates:

```text
.tactus/
  segno/
    jobs/                 installed task manifests
    state/
      business.sqlite3   typed workflow state and history
      lifecycle.sqlite3  cursors, occurrences, attempts, leases and fences
    triggers/             bounded invocation/result exchanges
```

`--sdk` names the directory containing `segno-flow.cabal`. When it is omitted,
Segno also checks `SEGNO_FLOW_SDK`, the current checkout, and the directory
next to the Clef SDK referenced by the project.

## Try the model-free active-window task

The repository example records the current foreground-window title once per
minute. It makes no model or network call. Copy it inside the target workspace,
compile it without executing it, install its typed manifest, and run one driver
turn:

```powershell
$source = Join-Path $repoRoot "segno-flow\examples\active-window\900_record_active_window.hs"
$script = Join-Path $projectRoot ".tactus\scripts\900_record_active_window.hs"
Copy-Item $source $script

tactus check --root $projectRoot --package segno-flow `
  --timeout-seconds 7200 $script
segno install --root $projectRoot $script
segno list --root $projectRoot
segno once --root $projectRoot
segno status --root $projectRoot --job record-active-window
segno history --root $projectRoot --state-key example.active-window --limit 20
```

The first check on a new Cabal installation may download Hackage dependencies
and compile Clef, Segno, cron, SQLite bindings, and Win32 support. Several
minutes of compiler output is normal. The example gives that cold build a
two-hour deadline; later checks reuse the Cabal store and build cache.

The first interval occurrence is due immediately. `once` plans currently due
occurrences, drains runnable work, prints a summary, and exits. To keep the
task active, run the local driver in a terminal you intend to leave open:

```powershell
segno driver --root $projectRoot --poll-seconds 5
```

Stop it with Ctrl+C. `--poll-seconds` is only a fallback driver wake interval;
the trigger plugin also returns its calculated next wake time. The plugin
itself never sleeps.

There are three independent timing controls:

- Direct `tactus check/run --timeout-seconds N` bounds its Cabal/GHC/runghc
  phase. The default is 1,800 seconds; `0` disables that direct deadline.
- `segno install/once/driver --task-timeout-seconds N` bounds each Tactus
  build/run phase started by Segno. The default is 1,800, the accepted range is
  1 through 604,800, and zero is rejected.
- `segno driver --poll-seconds N` controls only the maximum idle wait before
  polling again. It does not change the task's one-minute interval or its
  execution deadline.

Segno makes the Running lease longer than two task phases plus cleanup margin,
so increasing the task budget does not create an earlier lease handoff. A
timeout can still be ambiguous: if the driver cannot validate the task's
atomic result, it records `OutcomeUnknown` and does not retry that occurrence
automatically.

Real active-window capture currently supports Windows. Its title can include
private document, URL, or account text. The task checkpoints the business value
into local SQLite history; Tactus run evidence retains only a bounded diagnostic
summary of the plugin result. Installing this collection is an explicit opt-in.
Do not commit or share `.tactus/runs` or `.tactus/segno/state` without
inspection.

## Define a persistent task

The example's essential shape is ordinary Haskell:

```haskell
activeWindowTask :: PersistentTask WindowLog TimeEvent ActiveWindow
activeWindowTask =
  persistentTask
    "record-active-window"
    (gate notAlreadyRecorded
      (intervalTrigger (TriggerId "each-minute") 60000))
    (state (StateKey "example.active-window") (SchemaVersion 1) initialLog)
    recordWindow

main :: IO ()
main = runPersistentTask activeWindowTask
```

`Trigger state event` has two indices because `gate` reads both typed state and
typed event. Trigger leaves come from plugins; Clef composes them with:

- `mapTrigger` to transform the event type;
- `filterTrigger` to reject an event;
- `mergeTrigger` to route either source into one event type; and
- `gate` to reject an event using the current business state.

The installed JSON manifest contains only leaf identities, plugin names, and
open configuration. It cannot serialize arbitrary Haskell functions. On each
occurrence the script reconstructs the composition, decodes the selected leaf
payload, and applies map/filter/gate before entering the workflow.

There is deliberately no default `and` combinator. Combining two occurrences
requires an explicit correlation key and time window; those semantics are not
part of version one.

## Trigger occurrence and time semantics

Every occurrence carries:

- a stable trigger identity and occurrence identity;
- logical time and observed time;
- the trigger cursor;
- an idempotency key;
- an attempt number; and
- a typed payload.

The built-in `time.interval` and `time.cron` sources implement pure planning.
Given configuration, last durable cursor, current time, and a catch-up limit,
they return due occurrences and the next wake time. Segno inserts occurrences
durably before advancing the cursor. Cron uses UTC in version one.

This is at-least-once delivery: a claim or process crash can expose the same
occurrence again. A task or external plugin should use the occurrence
idempotency key when repeating an effect would be harmful. This does not mean
Segno retries an explicitly ambiguous result; `OutcomeUnknown` remains
terminal until an operator has reconciled external reality.

## Business state and lifecycle

`State state` describes the workflow's value, stable key, initial value, schema
version/migration, backend plugin, and compare-and-set behavior. The workflow
receives an immutable `StateHandle state`:

```haskell
checkpoint
  (CheckpointId "capture-active-window")
  handle
  nextState
```

A successful checkpoint returns a new handle with a new opaque revision. It is
a short transaction and becomes durable immediately; later workflow failure
does not roll it back. The workflow never holds a SQLite transaction while it
calls an agent or external process. Business-state CAS and the driver's later
lifecycle transition are separate commits, not a crash-atomic exactly-once
transaction across the two databases.

Lifecycle is a separate driver-owned value. It records scheduler states,
occurrence, attempt, lease and fencing information. User business state cannot
overwrite it. Final workflow decisions in version one are:

- `Ignore`;
- `Complete` with state transition and output;
- `Retry` with state transition, delay, and reason; and
- `Fail` with a structured business error.

A missing, malformed, mismatched, or ambiguous execution result becomes
`OutcomeUnknown`. The driver stops automatic decision-making for that
occurrence because a local process failure cannot prove an external provider
or effect did nothing.

## Plugin boundary

Built-in and third-party trigger plugins expose:

```text
describe  plan  poll  acknowledge  smoke
```

State plugins expose:

```text
describe  load  compare-and-set  append  history  smoke
```

They use the same `agenstro.plugin/v1` JSONL process boundary as other Tactus
plugins and can be implemented in Haskell, Rust, TypeScript, C#, or another
language. The first package ships `time.interval`, `time.cron`, `segno.state`,
and `system.active-window` Haskell hosts. The exact request, response, cursor,
acknowledgement, idempotency, and fencing rules are defined in the [Segno
plugin wire v1 reference](reference/segno-plugin-wire-v1.md).

## Inspect and test without a model

Machine-readable inspection is available for automation:

```powershell
segno list --root $projectRoot --json
segno status --root $projectRoot --json
segno history --root $projectRoot --limit 100 --json
```

From the Agenstro checkout, the default suite uses a virtual clock and fake
process boundaries. It neither waits for a wall-clock minute nor contacts a
model; CI separately type-checks the active-window task without capturing the
desktop:

```powershell
cabal build --builddir=Build/cabal all --enable-tests
cabal test --builddir=Build/cabal segno-flow:test:segno-flow-tests --test-show-details=direct
```

The suite covers strict plugin request identities, pure interval catch-up,
cross-job occurrence identity, occurrence-scoped checkpoint operations, stale
fencing tokens, package discovery, trigger-failure isolation,
`OutcomeUnknown` non-retry, and a virtual 0/60/120-second active-window task
that checkpoints three revisions and survives a simulated driver restart.

## Explicit non-goals

Segno `0.3` does not provide:

- exactly-once workflow, provider, or external-effect execution;
- distributed multi-node scheduling;
- serialized Haskell continuations;
- arbitrary workflow or `IO` replay;
- automatic rollback of checkpoints or external effects;
- an operator mutation command for resolving `OutcomeUnknown` occurrences;
- automatic migration of the removed Python/Rust Segno registry/state; or
- authentication, hostile-code sandboxing, or a network service API.

See [Architecture](architecture.md), [ADR-0004](adr/0004-haskell-segno-persistent-tasks.md),
and [Troubleshooting](troubleshooting.md) for the ownership and failure
boundaries.
