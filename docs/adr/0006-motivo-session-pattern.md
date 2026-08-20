# ADR-0006: Durable sessions and Motivo Studio's return channel

- Status: Accepted, staged
- Date: 2026-08-20
- Scope: Tactus session control and Motivo Studio
- Extends: ADR-0003 and ADR-0005
- Relates to: ADR-0004, which remains unchanged

## Context

Clef says what to compute, Tactus says how to execute it, Segno says when to
run it, and Motivo historically said only what to show. No component carried a
decision from a person back into a typed workflow boundary.

Some work cannot converge through more computation. A person must decide, but
often only after the system explains relevant findings and consequences. The
delay between question and answer may span days and reboots, so a live process,
thread, lease, or serialized Haskell continuation is the wrong durability
mechanism.

## Decision

Motivo Studio's positive charter is:

> Motivo is the boundary where machine state becomes human understanding, and
> human decisions become typed values.

Motivo remains a sandboxed projection. Tactus owns the workspace session store
and is the only writer; Motivo neither reads `.tactus/sessions` nor persists an
answer locally.

### Brief out, choice in

One `agenstro.session/v1` brief contains:

- sourced or visibly unsourced findings;
- exactly one question;
- two to six options labelled with comparable coordinates;
- the consequence and reversibility of the options;
- an optional planner-authored default; and
- both the remaining question surface and necessary floor.

Motivo returns only the session identity, turn, axis, option identity, and an
optional bounded note. It does not construct briefs, choose defaults, or apply
defaults on a timer.

### Session state is durable data

Tactus stores current state below `.tactus/sessions/<session-id>/session.json`
and replaces it atomically. The append-only decision transcript retains the
answered brief snapshot with each choice. No continuation or process survives
while a person is thinking.

The state machine is `planning`, `awaiting_answer`, `delivered`, or
`abandoned`. `pending` exists exactly in `awaiting_answer`.

### Answers use compare-and-set

Every answer supplies the turn and axis it was shown. Tactus locks the session,
rejects a stale turn, verifies that the option belongs to the pending question,
updates answers right-biased, and moves the document back to `planning`.

On `session_turn_stale`, Motivo refetches current state. Applying the old answer
to a new question would silently corrupt preference evidence.

### Planner purity and the Selective ceiling remain design requirements

The intended planner is pure over accumulating knowledge and answers. Provider
work belongs in separate inquiry handlers whose findings are persisted before
the planner runs again. The intended authoring ceiling is `Selective`, not
`Monad`, so a UI can show an over-approximated question surface and an
under-approximated necessary floor before executing the interview.

No incomplete planner API is exposed in this stage. The executable planner
registration and invocation contract must be decided before `session advance`
can be implemented safely.

## Current implementation boundary

This stage implements `session list`, `session show`, and `session answer`, plus
the corresponding validated Motivo views and return channel. The following are
explicitly deferred:

- `session advance`, until a planner executable/registration contract exists;
- a Clef `Selective` planner API and its surface/floor analysis;
- default application by an unattended runner;
- transcript projection and preference-norm mining;
- expiry; and
- the release policy for concurrent live sessions.

The storage and list contracts support multiple sessions, and Motivo provides
a picker rather than silently assuming only one exists.

## Consequences

The human delay becomes inert workspace data and survives process exit. The
renderer gains a narrow, typed inbound capability without gaining filesystem,
shell, scheduling, or runtime ownership. The staged boundary is useful for
validating projection and answer semantics, while avoiding a dead `advance`
button backed by a planner protocol that has not been designed.
