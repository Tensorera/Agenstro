# Studio control API v1

The Studio control API is the read-only, machine-facing boundary between the
Rust Tactus runtime and Motivo Studio. It is separate from
`agenstro.plugin/v1`: clients use it to inspect an initialized workspace and
page through validated trace events, not to implement plugins.

## Commands

```text
tactus studio inspect --root ROOT --run-limit N
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
- The on-disk `.tactus/runs` layout and `runtime.json` are runtime internals.
  Studio clients must not read them directly.
- Motivo starts Tactus with an argument array and `shell: false`; the renderer
  receives only validated projections and never owns the workspace root.

The current reference limits and DTOs live in the Rust
`tactus-runtime::studio` module. Changes that break these rules require a new
control or projection API version.
