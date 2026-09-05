---
title: Motivo task method and execution ownership
status: working decision record
owners: [architecture, motivo]
last_verified: 2026-09-05
applies_to: "Motivo Studio 0.3.0 and Tactus Rust 0.3.0"
platforms: [windows, ubuntu]
---

# ADR-0007: Motivo owns task method; Tactus owns execution

- Status: Accepted
- Date: 2026-09-05
- Scope: Motivo task work and its boundary with Clef/Tactus
- Updates: ADR-0006's projection-only Motivo charter; its existing session
  storage and answer protocol remain unchanged
- Leaves unchanged: ADR-0004 and Segno implementation

## Context

Making a reusable Haskell program is useful when the work has a stable,
repeated procedure. Requiring that program before a simple engineering task
adds authoring and compilation work, and can force decisions before the agent
has gathered the observations that justify them.

The provider adapter already invokes a complete native coding agent. Task
method belongs above that execution boundary; it need not become a domain
policy in Clef or an additional supervisor inside Tactus.

## Decision

Motivo owns a replaceable task method, local task history, and the user
interface. The default method offers investigate, try, integrate, and conclude
as available actions. It does not require a fixed order, a global plan, extra
roles, or Haskell generation for every task.

Motivo sends each provider request through the existing Tactus dispatch
protocol. Tactus resolves provider configuration and supervises the native
agent. Clef remains the typed composition option, and Segno remains the
persistent scheduler. No new Tactus task-loop or domain-validation protocol is
introduced.

Each continuation has a budget of four provider calls by default and at most
twenty. A call is a native agent episode, not one internal model/tool step.
The lead may request up to three independent investigator calls; they count
against the same budget, with a lead integration call reserved before branch
dispatch. Investigators receive separate contexts and are instructed not to
edit. The working environment is shared, so this is neither a read-only
sandbox nor isolation for concurrent writes. Dependent edits stay sequential
with the lead.

The method can use existing project tests and plugins. A lead may create a
small project plugin and fixtures if missing observation capability directly
blocks progress, then invoke it through Tactus. Test behavior and success
criteria belong to the project. Motivo's fixed report schema governs the
handoff format, not what universally makes a task correct.

An optional `.motivo/METHOD.md` replaces default method text while leaving the
report protocol intact. The application atomically saves `motivo.task/v1`
documents at `.motivo/tasks/<uuid>.json`, containing goal, constraints,
provider, notes, call timing, and reports. It does not write Tactus session
documents or Segno state. Task records may contain business content and are
distinct from Tactus's redacted diagnostic journals.

Task states are ready, running, paused, needs_input, completed, failed, and
outcome_unknown. Pause waits for current calls to finish. Budget exhaustion
preserves a handoff. Process interruption and an unusable post-execution
report do not trigger automatic retry. Continuing an unknown outcome requires
a user note describing the reconciliation decision. Fresh calls receive
bounded recent reports and source references; no native session or Haskell
continuation is serialized.

Tasks is the default interface. Workflow retains explicit source selection;
Sessions retains the earlier Tactus list/show/answer compatibility surface.
The session planner and `session advance` remain outside this change.

## Consequences

Simple work can proceed directly, while useful repeated procedures can still
be expressed with Clef. Method experiments can change project guidance without
changing the process kernel or imposing a domain-specific correctness rule.

The renderer keeps named, validated IPC and no arbitrary shell/filesystem
authority. Electron main gains only the task-method/store responsibilities
described above. Tactus still owns external invocation supervision.

A valid report is not a proof that its claims are true. `completed` means the
lead reports delivery; recorded checks remain reported observations. The
method does not update model weights, promise higher model capability, provide
exactly-once effects, or restore a workspace. Capability and efficiency claims
require separate evaluation against task outcomes and direct-agent baselines.
