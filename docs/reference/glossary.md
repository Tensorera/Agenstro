---
title: Agenstro glossary
status: alpha
owners: [documentation]
last_verified: 2026-09-05
applies_to: "Agenstro 0.3"
platforms: [windows, ubuntu]
---

# Agenstro glossary

This glossary gives one canonical meaning to terms shared by Clef, Tactus,
Segno, plugins, and Motivo Studio.

## Components

**Agenstro**

The whole project and its shared contracts. The name combines agent-oriented
work with orchestration.

**Clef**

The Haskell EDSL and runtime library for typed workflow composition. Like a
musical clef, it establishes the type frame in which later values are read.

**Tactus**

The Rust command/runtime for `.tactus` workspaces, script selection, process
supervision, event routing, and diagnostic journals. The name refers to the
measured pulse that turns a score into execution.

**Segno**

The Haskell persistent-task driver. It owns trigger time, cursors, occurrences,
attempts, leases, fences, and SQLite state while delegating actual workflow
execution to Tactus. A segno is a navigation mark in a score.

**Motivo Studio**

The TypeScript task method and React/Electron interface over Tactus. It chooses
task-level actions and owns `.motivo` task reports while Tactus supervises
execution and Segno retains persistent scheduling.

## Workflow terms

**Workflow**

A Clef `Workflow a`: a typed Haskell computation returning `a` on success.

**Task**

In Clef, a provider-shaped definition that renders a typed input as a prompt
and decodes final text into a typed output. A **Motivo task** is a separate
application record for a goal, constraints, provider, notes, and report history;
it need not contain a Clef program or a Segno persistent task.

**Operation**

A typed call to one method in the Tactus `[effects]` registry.

**Plugin**

An open typed call to one method in the `[plugins]` registry. In wire-level
discussion, “plugin” can also mean any one-shot executable in the provider,
effect, generic, trigger, or state categories.

**Provider**

A plugin shaped around a coding-agent prompt and final text. Provider names,
models, effort, and credentials are runtime concerns rather than Haskell types.

**Effect**

A named external capability. An effect can observe or change systems outside
the typed workflow; it is not automatically transactional or reversible.

**Entry**

A runnable `.hs` or `.lhs` file whose basename starts with a valid three-digit
order and lowercase slug, such as `020_review.hs`.

**Helper**

A Haskell source below `.tactus/scripts` that does not match the runnable entry
naming rule. Tactus checks helpers but does not select them for default run.

## Runtime terms

**Task method**

Motivo's replaceable guidance for selecting useful actions from current
evidence. The default offers investigate, try, integrate, and conclude without
requiring their order or use. `.motivo/METHOD.md` can override the guidance,
but not the structured-report protocol.

**Task report / handoff**

A structured agent account of focus, findings, uncertainty, decisions,
artifacts, checks, and the next action, stored in `.motivo/tasks/<uuid>.json`.
It supports bounded context for later calls. It is neither independent
verification nor a serialized agent continuation.

**Lead / investigator**

The lead carries the Motivo task and its dependent edits. Optional investigators
answer independent questions in separate prompts. They share the working
environment and are instructed to avoid writes; this is not an enforced
read-only capability or parallel-write isolation.

**Provider-call budget**

The number of native agent episodes Motivo may request in one continuation:
four by default, at most twenty, including investigation branches. One episode
may contain multiple internal model/tool steps. A budget is not a token, price,
or wall-clock guarantee.

**Completed task**

A Motivo task whose lead reports delivery. The application checks report
structure, not whether the delivered work is correct. This is separate from a
successful Tactus process result or a project test result.

**Invocation**

One plugin request/process lifecycle with zero or more event frames and exactly
one authoritative terminal frame when the protocol completes normally.

**Run**

A Tactus command execution or supervised invocation represented by an opaque
run ID and diagnostic journal. A run is not necessarily a whole multi-script
business transaction.

**Event**

A non-terminal observational plugin frame. Events may be streamed, aggregated,
or dropped under bounded pressure and never replace the terminal result.

**Terminal**

The unique plugin result frame declaring structured success or failure.

**OutcomeUnknown**

The classification used when external work may have completed but no
trustworthy terminal outcome is available. It must not be treated as known
failure or retried blindly.

**Journal / run evidence**

Bounded structured diagnostic events and a terminal summary below
`.tactus/runs`. Journals aid investigation; they are not replay state, backup,
or proof of all external side effects.

**Presentation**

The human projection consisting only of `[state]`, `[info]`, `[warning]`, or
`[error]` plus natural language.

**Observation degradation**

A failure or budget overrun in a journal, callback, stderr reader, observer, or
UI projection. It is recorded separately and does not replace a known
authoritative terminal result.

## Persistent-task terms

**Persistent task**

A Segno binding of typed trigger, versioned business state, Clef workflow, and
execution policy. Persistence means the driver can create later occurrences;
it does not mean a suspended Haskell continuation is serialized.

**Trigger**

A typed source of candidate events. A leaf plugin plans/polls occurrences;
Haskell composes leaves with map, filter, merge, and state-aware gate.

**Occurrence**

One durable trigger delivery with identity, logical time, observed time,
cursor, idempotency key, attempt, and typed payload.

**Attempt**

One execution try for an occurrence. A retry increments the attempt while
preserving occurrence identity.

**Logical time**

When an occurrence was supposed to happen according to its trigger.

**Observed time**

When the Segno driver actually detected or processed the occurrence.

**Cursor**

Durable trigger-source progress used to plan later occurrences without relying
on a sleeping plugin process.

**Lifecycle state**

Segno-owned scheduling state such as Ready, Claimed, Running, Waiting, or
OutcomeUnknown. User workflow code cannot overwrite it.

**Business state**

Typed, versioned user state accessed through `State state` and short
compare-and-set checkpoints. It is separate from lifecycle state.

**Lease**

A bounded claim interval during which one driver attempt owns an occurrence.

**Fencing token**

A token checked by state transitions/checkpoints to reject a stale attempt
after its claim is no longer authoritative.

**Checkpoint**

A successful short business-state compare-and-set. It is a durable fact and is
not rolled back if later workflow work fails.

**At-least-once**

A delivery model that can repeat work around failures. Idempotency and fencing
reduce risk but do not create exactly-once external effects.

## State-transition terms

**`state_before`**

The authoritative state before one decision.

**Trigger (transition field)**

What caused reevaluation: request, event, timer, internal result, or control.

**Guard**

The condition evaluated, whether it passed, and the reason that justifies the
transition.

**`state_after`**

The authoritative state committed after the guard allowed the transition.
