---
title: Logs, state transitions, and run evidence
status: alpha
owners: [tactus, motivo]
last_verified: 2026-08-17
applies_to: "Clef/Tactus/Motivo 0.3"
platforms: [windows, ubuntu]
---

# Logs, state transitions, and run evidence

Agenstro separates what a person should read from what a diagnostic tool should
retain. Human surfaces show short natural-language state and severity messages;
the persistent layer records bounded structured evidence about why state
changed.

## The execution model: `G(s, i, t, h)`

Treat one program decision as:

```text
next = G(state, input, time, history)
```

This model unifies one-shot requests, filesystem events, scheduled tasks,
provider results, and operator cancellation without calling all of them “logs.”

### State (`s`)

State is the currently authoritative condition at one layer:

- Clef workflow and plugin-call states such as ready, running, succeeded,
  failed, cancelled, or outcome unknown;
- Tactus command/process state, including preparing a Haskell run;
- Segno lifecycle state (`Dormant`, `Ready`, `Claimed`, `Running`, `Waiting`,
  `Succeeded`, `Failed`, `OutcomeUnknown`);
- versioned Segno business state, which is deliberately separate from
  lifecycle state; and
- Motivo action projection state, which mirrors Tactus rather than creating a
  new runtime state machine.

### Input (`i`)

Input explains what requested evaluation:

- a human command or provider prompt (`request`);
- a plugin, filesystem, or external notification (`event`);
- a scheduled occurrence (`timer`);
- a completed child operation (`internal_result`); or
- cancellation, retry policy, or another runtime instruction (`control`).

Payload values belong to the typed workflow or plugin contract. The diagnostic
record should identify the trigger without copying arbitrary provider content.

### Time (`t`)

Time influences deadlines, scheduled occurrences, lease expiry, debounce or
throttle behavior, and ordering. Segno distinguishes logical time—the time an
occurrence was supposed to happen—from observed time—the time the driver
actually detected it.

Wall-clock time is evidence, not a substitute for sequence or identity. Run
events also carry a monotonically increasing sequence within one journal.

### History (`h`)

History includes only durable facts that the current decision is allowed to
consult:

- current source and `.tactus/tactus.toml` configuration;
- prior typed values within the running Clef workflow;
- Tactus run diagnostics used by people and tools for investigation; and
- Segno trigger cursors, occurrence lifecycle, attempts, fencing tokens, and
  business-state revisions.

Tactus run journals are not replay state. Segno may consult its own databases,
but Clef does not silently substitute a recorded provider result for a new
call.

## Human presentation

Shells and Motivo Studio use exactly four labels:

```text
[state] A provider invocation started.
[info] Tactus discovered three runnable workflow entries.
[warning] Some workspace paths could not be observed and were omitted.
[error] The provider executable could not be started.
```

| Label | Meaning |
| --- | --- |
| `[state]` | An actual lifecycle transition or terminal state |
| `[info]` | Useful progress or a neutral result that is not a transition |
| `[warning]` | Degraded observation, ambiguity, or a condition needing attention without claiming known failure |
| `[error]` | A known operation/configuration failure |

The message is bounded natural language. Provider JSON, native stderr, stack
traces, event payloads, argv, and structured frames are technical detail and
must not be printed as if they were user-log messages.

Compiler output and trusted workflow stdout/stderr are separate process output.
They may still be visible, but their text is not automatically assigned an
Agenstro severity.

## Durable transition record

A real transition records four mandatory explanations:

```json
{
  "state_before": "ready",
  "trigger": {
    "kind": "request",
    "source": "tactus.run",
    "code": "run.requested"
  },
  "guard": {
    "condition": "script selection and runtime preparation succeeded",
    "passed": true,
    "reason": "the selected entry is ready to execute"
  },
  "state_after": "running"
}
```

The interpretation is fixed:

1. `state_before`: what was authoritative before evaluation;
2. `trigger`: what requested or supplied the change;
3. `guard`: which condition was evaluated and why it allowed the change; and
4. `state_after`: what became authoritative after the decision.

Trigger kinds are `request`, `event`, `timer`, `internal_result`, and
`control`. Stable codes and bounded correlation context may accompany the
transition. Arbitrary free text is summarized before it becomes durable
presentation.

Progress, provider thinking, compiler lines, and observer warnings are not
state transitions. They use message or diagnostic event records.

## Terminal authority

The unique valid plugin terminal frame is authoritative for a supervised
invocation. Failure in a journal writer, callback, Studio projection, or stderr
reader is observation degradation; it must not turn a known plugin success or
failure into another business outcome.

When no trustworthy terminal exists after a provider may have acted, the
correct state is `OutcomeUnknown`. That is not another spelling of failure:

- the external action may have completed;
- Agenstro cannot safely assert success or failure; and
- automatic retry could duplicate a side effect.

The human surface therefore shows the state plus a warning explaining that
reconciliation is required.

## Run journal layout

Supervised plugin invocations and Tactus run operations write unique directories
below:

```text
.tactus/runs/<run-id>/
  events.jsonl
  summary.json
```

`events.jsonl` contains ordered `agenstro.trace/v1` diagnostic events.
`summary.json` is atomically published when a terminal summary is available.
An absent summary means the record is open, abandoned, or incomplete; a reader
must not invent a terminal state from the last event.

Tactus keeps low-priority events bounded. Excess progress can be aggregated or
dropped with an `events_dropped` diagnostic while preserving capacity for
terminal evidence. Writer degradation is recorded separately when possible.

## Privacy and redaction

The journal is designed for diagnosis, not for retaining full prompts or
results. Recognized sensitive fields, provider raw output, terminal values,
native stderr, and open error details are reduced to bounded summaries, hashes,
counts, or stable codes before persistence.

This is not a secrecy guarantee. File names, model identifiers, error codes,
path metadata, hashes, timestamps, and bounded unknown diagnostic fields can
still reveal information. Review run directories before sharing them and keep
credentials out of prompts, config, source, and environment dumps.

Temporary Clef runtime configuration is created outside the run journal and
removed after the command. Crash leftovers use a private prefix, an ownership
lease, and bounded stale cleanup; they are internal files, not published
evidence.

## Motivo projection

Motivo asks Tactus for versioned, redacted workspace and run projections. It
does not parse TOML or read `.tactus/runs` directly.

Canonical `presentation` fields become the four visible labels. Legacy or raw
event data stays under closed technical details. If the desktop output budget
is exceeded, Motivo keeps draining the child process, emits one warning, drops
additional projection text, and preserves the real Tactus exit outcome.

## What to inspect after a failure

Use this order:

1. read the final human `[state]`, `[warning]`, or `[error]` message;
2. identify the command and run ID;
3. inspect `summary.json` for the authoritative diagnostic outcome;
4. page structured events through `tactus studio events RUN_ID` or Motivo;
5. for `OutcomeUnknown`, inspect the external provider/system and workspace;
6. for Segno, also inspect lifecycle and business-state history; and
7. retry only after the idempotency and side-effect risk is understood.

Use [Troubleshooting](troubleshooting.md) for symptom-specific procedures and
[Workspace operations](operations.md) for retention and backup guidance.
