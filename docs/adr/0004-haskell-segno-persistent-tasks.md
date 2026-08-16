# ADR-0004: Haskell Segno persistent tasks and open backends

- Status: Accepted
- Date: 2026-08-16
- Scope: Segno Flow 0.3
- Extends: ADR-0003

## Context

Clef `0.3` made workflows ordinary typed Haskell programs and moved provider
and effect implementations behind an open process protocol. That solves
one-shot composition, but not a task that must still be triggerable after the
workflow process has exited.

The previous Segno experiment combined Python packaging, a Rust scheduler,
leases, and an unrelated desktop surface. It also blurred scheduling and
replay. The replacement needs a smaller centre: typed trigger composition and
typed business state in Haskell, durable single-machine scheduling, and the
same language-neutral plugin boundary used by Clef and Tactus.

## Decision

### 1. Segno is Haskell

Both the public persistent-task API and the single-node Segno driver are
Haskell. Clef exposes the compact task boundary; the `segno-flow` Cabal package
owns planning, lifecycle, persistence, plugin hosts, and the `segno` command.
There is no Python shim and no Segno Rust crate in the current path.

The central relationship is:

```text
Trigger state event + State state + Clef Workflow decision
                         |
                         v
                    PersistentTask
```

`Trigger state event` deliberately has two type parameters. The state index is
needed for an honest, statically typed `gate`; pretending this is only
`Trigger event` would hide the state dependency.

### 2. Haskell composes triggers; plugins provide leaves

A trigger leaf has a stable identity, a plugin name, open configuration, and a
typed payload decoder. Clef supplies only these structural combinators:

- `mapTrigger` transforms a payload;
- `filterTrigger` rejects a payload;
- `mergeTrigger` routes either source into one event type; and
- `gate` checks the current typed business state and event.

Arbitrary Haskell functions cannot be serialized. The installed manifest
therefore contains only trigger leaves. Segno plans a leaf, starts the script
through Tactus, and the script reconstructs and evaluates the Haskell
composition before calling its handler.

There is no default Boolean `and` trigger. Correlating two event streams needs
an explicit key and time window and is deferred until those semantics are
designed.

### 3. Time plugins plan and never sleep

The built-in `time.interval` and `time.cron` plugins implement the open trigger
methods `describe`, `plan`, `poll`, `acknowledge`, and `smoke`. Planning is a
pure calculation over configuration, durable cursor, and current time. The
plugins report due occurrences and the next wake time; they do not sleep or
own a scheduler thread.

The Segno driver owns waiting, observed time, cursor persistence, occurrence
insertion, and wake-up. This makes time behavior testable with a virtual clock
and prevents a plugin process exit from losing the schedule. Version one cron
uses UTC.

### 4. Business state and lifecycle state never share a value

`State state` declares a stable key, schema version, initial value, migration,
backend plugin, and compare-and-set conflict behavior. A workflow receives an
immutable `StateHandle state`; an explicit successful checkpoint returns a new
handle with the new opaque revision.

The SQLite backend stores business state and its append-only events separately
from Segno-owned lifecycle records. User code cannot assign scheduler states
such as `Running` or alter attempts, leases, or fencing tokens through its
typed business state.

Lifecycle includes `Dormant`, `Ready`, `Claimed`, `Running`, `Waiting`,
`Succeeded`, `Failed`, and `OutcomeUnknown`. Updates use short transactions and
opaque revisions rather than holding a database transaction across a workflow
that may run for hours. A committed checkpoint is durable fact and is not
rolled back if a later step fails.

### 5. A workflow returns an explicit decision

One occurrence supplies trigger identity, occurrence identity, logical and
observed time, cursor, idempotency key, attempt, typed payload, and typed state
handle. The workflow returns one of:

- `Ignore`;
- `Complete`, with a final state transition and output;
- `Retry`, with a state transition, delay, and reason; or
- `Fail`, with a structured business failure.

The first release deliberately omits `Wait` and `Cancel`. They can be added
without serializing a Haskell continuation: a future decision can persist
state and wait for another trigger.

### 6. Segno drives; Tactus executes

Segno is the long-lived local driver because triggers must exist while no
workflow is running. For each occurrence it writes a bounded invocation
document and invokes the installed Haskell script through `tactus run` with
the `clef-sdk` and `segno-flow` packages exposed. The script writes an atomic,
bounded result document for the driver to validate and commit.

Trigger, state, and domain plugins are still one-shot
`agenstro.plugin/v1` processes and may be written in any language. The built-in
time, SQLite-state, and Windows active-window implementations happen to be
Haskell. The driver reaches them through Tactus, so configuration, process
supervision, and protocol validation keep one boundary.

Segno state lives inside the selected project:

```text
.tactus/
  scripts/
  runs/
  segno/
    jobs/
    state/
    triggers/
```

### 7. Delivery is at least once

Occurrence identity and idempotency keys are stable, insertion is durable,
and claims carry attempts, leases, and fencing tokens. A process or transport
failure may leave the external outcome unknowable. Segno records
`OutcomeUnknown` instead of assuming it is safe to run an effect again.

This design does not claim exactly-once execution. Workflow and plugin authors
must use idempotency keys, checkpoints, and domain-specific reconciliation
when duplicate external effects matter.

## Consequences

- GHC checks trigger payloads, state-aware gates, migrations, state
  transitions, and workflow outputs before installation.
- Trigger and state implementations remain replaceable without expanding the
  Clef core or requiring Haskell on the plugin side.
- Interval/cron behavior, missed occurrences, restarts, cursor advancement,
  and duplicate suppression can be tested without waiting on wall-clock time
  or contacting a model.
- SQLite is the first local backend, not a universal storage abstraction or a
  distributed consensus system.
- A business-state CAS and lifecycle transition are separate durable commits;
  the two physical SQLite databases do not form an exactly-once crash-atomic
  workflow transaction.
- The active-window example is local and model-free, but window titles can be
  sensitive and remain an explicit opt-in collection.
- Segno is persistent scheduling, not workflow replay. It does not substitute
  recorded plugin results or intercept arbitrary Haskell `IO`.
- Version one has no exactly-once guarantee, distributed multi-node driver,
  automatic rollback of external effects, serialized continuation, or
  arbitrary workflow replay.
