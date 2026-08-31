---
title: Studio control API v1
status: alpha
owners: [tactus, motivo]
last_verified: 2026-08-31
applies_to: "tactus.studio/v1 and tactus.control/v1"
platforms: [windows, ubuntu]
---

# Studio control API v1

The Studio control API is the read-only, machine-facing boundary between the
Rust Tactus runtime and Motivo Studio. It is separate from
`agenstro.plugin/v1`: clients use it to inspect an initialized workspace and
page through validated trace events, not to implement plugins.

Human-decision reads and answers share the `tactus.control/v1` envelope but
have their own bounded domain contract; see
[Session document and control API v1](session-control-v1.md).

## Commands

```text
tactus studio inspect --root ROOT [--exact-root] --run-limit N
tactus studio events RUN_ID --root ROOT --after SEQ --limit N --max-bytes B
```

Both commands write exactly one UTF-8 JSON document to standard output. A
successful response has this envelope:

```json
{
  "api": "tactus.control/v1",
  "command": "studio.inspect",
  "status": "completed",
  "data": {
    "api": "agenstro.studio/v1"
  }
}
```

A control error uses the same envelope and a stable, redacted failure:

```json
{
  "api": "tactus.control/v1",
  "command": "studio.events",
  "status": "error",
  "error": {
    "code": "invalid_run_id",
    "message": "The supplied run identifier is invalid."
  }
}
```

`completed` means Tactus produced a domain result. It does not mean that every
doctor check passed or that a traced plugin invocation succeeded.

`--exact-root` rejects upward workspace discovery with
`workspace_root_mismatch`. Motivo always uses it so Electron main's private
redaction root is exactly the workspace Tactus inspected; the error never
reveals the discovered parent path.

## Workspace snapshot

`studio.inspect` returns:

- `generatedAtUnixMs` and other counters as decimal strings;
- `workspace.name`, never the absolute workspace path;
- `health.ok` and factual doctor checks with redacted details;
- deterministically ordered scripts as `relativePath`, optional `order`, and
  `runnable`;
- separate provider, effect, and generic plugin registries;
- a bounded list of recent run projections.

Registry entries expose the key, namespace, availability, default-provider
status, optional model and effort, and observer status. They do not expose
plugin command arrays, arbitrary options, credentials, runtime instructions,
or generation prompts.

The run limit is between 1 and 200. Tactus examines at most 2,000 run
directories for one snapshot. An unreadable or corrupt individual run is
reported as corrupt without preventing the rest of the workspace snapshot.

## Event pages

`studio.events` accepts only the opaque `runId` returned by `studio.inspect`.
It rejects separators, traversal components, and non-plain trace paths. The
response contains:

- the current compact `run` projection;
- events whose sequence is greater than `after`;
- `nextAfter`, suitable for the next bounded request;
- `complete`, which is true only after a valid terminal summary and complete
  end-of-file read;
- `integrity`: `ok`, `partial`, or `corrupt`;
- a terminal `summary` when Tactus has atomically published one.

Event `kind` and `data` are intentionally open. Clients must retain or display
unknown event kinds and must not infer replay semantics from them. A missing
summary means the run is open or incomplete; it is not proof that a process is
still alive.

An event may additionally contain a backward-compatible human projection:

```json
{
  "kind": "runtime.state_transition",
  "presentation": {
    "category": "state",
    "message": "provider:codex method generate started."
  },
  "data": {
    "state_before": "ready",
    "trigger": {
      "kind": "request",
      "source": "tactus.cli",
      "code": "plugin.invocation_requested"
    },
    "guard": {
      "condition": "plugin resolved and request validated",
      "passed": true,
      "reason": "The configured plugin and invocation request passed runtime validation."
    },
    "state_after": "running"
  }
}
```

`presentation.category` is exactly `state`, `info`, `warning`, or `error`.
`message` is bounded natural language with embedded newlines flattened. The
`state` category is reserved for a real lifecycle transition; every such event
uses `runtime.state_transition` and the four-part `state_before`, `trigger`,
`guard`, `state_after` diagnostic. Progress events do not claim a transition.

Clients should render `presentation` directly and keep `kind` plus `data` as
collapsed technical evidence. Older trace-v1 events may omit `presentation`;
clients must not guess a public severity or message from the open event kind or
payload. They may expose that legacy event only as technical details.

The underlying trace is already a diagnostic persistence projection. Prompt,
provider raw/text/content, credential-like, options, environment, workspace,
and path fields are replaced with bounded byte-count/SHA-256 summaries;
terminal success values and native stderr are summarized rather than persisted
verbatim. This redaction does not turn the trace into replay state, and bounded
errors or path metadata may still be sensitive.

The event count limit is 1–1,000, the page byte limit is 1–8 MiB, each JSONL
record is limited to 1 MiB, and a summary is limited to 2 MiB. A partial final
line or exhausted page budget yields `partial`; a complete malformed record,
sequence gap, mismatched API, run id, or summary count yields `corrupt`.

## Compatibility rules

- JavaScript-facing timestamps, sequences, elapsed times, and counts are
  decimal strings so values above `2^53` remain exact.
- Clients must reject an unknown top-level `api` or command envelope, while
  tolerating additional fields and unknown event kinds inside a recognized
  version.
- The optional event `presentation` field is additive within trace v1. Its four
  categories are closed; event `kind` and `data` remain open.
- The on-disk `.tactus/runs` layout and the temporary Clef runtime
  configuration are runtime internals. Runtime configuration is removed after
  the supervised command; Studio clients must not read either implementation
  detail directly.
- Motivo starts Tactus with an argument array and `shell: false`; the renderer
  receives only validated projections and never owns the workspace root.

The current reference limits and DTOs live in the Rust
`tactus-runtime::studio` module. Changes that break these rules require a new
control or projection API version.
