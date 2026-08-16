# Local plugin protocol v1

`agenstro.plugin/v1` is the small language-neutral process boundary shared by
Clef and Tactus. Providers and effects have convenience registries, while
`[plugins]` accepts arbitrary domain plugins; all three use the same transport.

## One-shot process contract

For each call the runtime:

1. starts the configured argv in the workflow workspace without a shell;
2. inherits the caller environment, including native provider credentials;
3. writes one UTF-8 JSON request followed by LF and closes stdin;
4. reads zero or more UTF-8 JSONL event frames and exactly one terminal result
   from stdout; and
5. reads stderr as human diagnostics, never as protocol data.

Version 1 is deliberately one-shot. There is no daemon, socket discovery,
authentication handshake, persistent session, or service registry hidden in
the ABI.

In the normal Tactus/Clef path, generated runtime configuration points Clef at
the Rust `tactus dispatch` subcommand. Tactus then starts the real plugin in a
Unix process group or Windows Job Object, validates and forwards frames as they
arrive, and journals the invocation. Clef parses those forwarded lines
incrementally and enqueues them to its bounded, serial `EventSink` worker before
the terminal result. `runWorkflow` boundedly flushes the final value/evidence;
sink failure is a runtime-projection failure, not an unknown provider outcome.

The Rust supervisor uses bounded requests, frames, total stdout, retained
stderr, frame count, pending-event queue, and sink-delivery time. An isolated
worker performs sink I/O, so a blocked consumer cannot stop deadline or
cancellation polling. Its default wall-clock deadline is 1,800 seconds; public
CLI options may override it, and a supported value of zero disables it
deliberately. Deadline, cancellation, protocol failure, queue overflow, and a
stalled sink terminate the owned process group. Windows Job Objects contain the
nested tree; on Unix, a process that deliberately creates a new session can
escape process-group containment. Local termination cannot prove that a remote
provider did not already complete work.

## Request

```json
{
  "api": "agenstro.plugin/v1",
  "id": "runtime-generated-correlation-id",
  "method": "invoke",
  "params": {}
}
```

The request object has exactly these top-level fields:

- `api` must be `agenstro.plugin/v1`;
- `id` is a correlation string (a signed integer is also accepted by the Rust
  decoder; new integrations should prefer a string);
- `method` is a non-empty plugin-defined operation; and
- `params` is an open JSON object owned by that method.

Provider-specific options must be preserved rather than translated into a
closed model/effort enumeration.

## Event frames

An event is non-terminal:

```json
{
  "type": "event",
  "id": "runtime-generated-correlation-id",
  "event": {
    "type": "progress",
    "message": "step 2 of 4"
  }
}
```

`event.type` and the remaining event fields are open. A runtime must preserve
unknown event subtypes and must not confuse them with new top-level frame
kinds. Plugins should terminate and flush every event line immediately;
partial lines are not exposed to an event sink.

Events are evidence/progress. They are recorded and projected to observers,
but do not enter the typed workflow value graph.

## Terminal result

Success requires `ok=true`, a present `value` (which may itself be JSON null),
and no `error`:

```json
{
  "type": "result",
  "id": "runtime-generated-correlation-id",
  "ok": true,
  "value": {"answer": 42}
}
```

Failure requires `ok=false`, no `value`, and a structured error:

```json
{
  "type": "result",
  "id": "runtime-generated-correlation-id",
  "ok": false,
  "error": {
    "code": "provider_exited",
    "message": "native provider exited before a final response",
    "details": {}
  }
}
```

`error.code` and `error.message` are strings; `details` is optional open JSON.
A valid structured plugin failure remains distinct from malformed protocol,
deadline, cancellation, and an incoherent process exit.

Once a plugin process has started, a deadline, cancellation, broken transport,
invalid/missing terminal frame, output limit, or contradictory process exit is
reported as `error.code = "outcome_unknown"`, with the local cause in
`error.details`. This conservative rule applies to every plugin-defined method,
not only provider `invoke`: an arbitrary method may already have performed an
external side effect. A caller must not infer that retrying is safe merely from
the method name.

## Strictness and lifecycle

All stdout data must be UTF-8 protocol JSONL and use the active request ID. The
current strict decoders reject:

- duplicate object keys at any depth;
- malformed JSON; integers outside signed-64-bit through unsigned-64-bit range;
  or floating values that overflow, are non-finite, or underflow a nonzero
  literal to zero;
- an unknown request API, empty method, or non-object params;
- an unknown top-level frame type;
- a correlation mismatch;
- success without `value` or with an `error`;
- failure without `error` or with a `value`;
- a second terminal result or any frame after the terminal result;
- process exit without a terminal result; and
- configured transport-limit violations.

Diagnostics, banners, native progress bars, and debug prints belong on stderr.
An adapter that writes them to stdout corrupts its protocol stream.

## Common discovery and health methods

Bundled plugins implement:

- `describe`: implementation name/version, kind, methods/operations, open
  option schema, and observed capabilities;
- `smoke` with `live=false`: resolve the native executable and obtain its
  version without a model prompt; and
- `smoke` with `live=true`: perform the adapter's documented minimal external
  request.

General plugins may define any other methods. Call them from Clef with
`jsonPlugin name method`/`rawPlugin`, or from Tactus:

```powershell
tactus plugin-call calculator add --namespace plugin --params '{"left":19,"right":23}'
```

## Provider method

The bundled provider adapters expose `invoke` with open parameters shaped like:

```json
{
  "prompt": "...",
  "workspace": "D:/absolute/project",
  "model": null,
  "effort": "high",
  "options": {},
  "extra_args": []
}
```

OpenCode uses `variant` as its native reasoning selector and accepts effort as
a compatibility fallback. Adapters emit normalized native-stream events and a
terminal object containing at least the resulting text and process/provider
metadata. Session reuse is not implicit.

## Observer lifecycle

An effect configured with `observe_invocations = true` may implement:

- `observe.begin {workspace, invocation, context, options}` returning an opaque
  JSON begin value; and
- `observe.end {workspace, invocation, context, options, begin, outcome}`
  returning evidence.

Controllers begin in stable configuration order and end in reverse order.
Provider output and controller evidence remain separate records. Cleanup is
best-effort when cancellation or process loss makes an earlier begin result
unknown; this protocol does not promise exactly-once effects.

## Built-in provider adapters

The Rust Tactus binary launches native providers in their most permissive
documented non-interactive modes:

- Codex: `codex exec --dangerously-bypass-approvals-and-sandbox --json ...`;
- Claude Code: `claude -p --dangerously-skip-permissions --output-format
  stream-json ...`; and
- OpenCode: `opencode run --auto --format json ...` with an inline
  `permission=allow` configuration.

OpenCode's `--auto` approves ask decisions but cannot override every explicit
deny or managed policy. Its `describe`/`smoke` result therefore reports
`full_bypass=false` instead of claiming permission parity.

Provider login, credentials, billing, model availability, and policy are owned
by the native CLI. The protocol adds no authorization layer.

## Built-in `workspace.paths`

The observational effect implements:

- `snapshot {workspace, options}` -> opaque `{snapshot_id}`;
- `diff {workspace, before:{snapshot_id}, after:{snapshot_id}}` -> path lists;
- `forget {workspace, snapshot_id}` -> `{forgotten}`; and
- the observer begin/end lifecycle above.

`observe.end` is idempotent for the opaque begin token. The first completed
delta is committed atomically; concurrent or crash-recovery retries return the
same persisted value after validating the workspace and invocation. These
metadata-only completion records become eligible for bounded cleanup after 24
hours and are not a replay log.

Its final delta has this shape:

```json
{
  "added": [],
  "modified": [],
  "deleted": [],
  "type_changed": []
}
```

Entries are slash-separated workspace-relative paths. Internally, snapshots
compare path kind, byte size, and SHA-256; public results do not contain file
contents. The effect excludes `.git`, its internal state/run data, and common
build trees (`target`, `node_modules`, `build`, and `dist-newstyle`). It does not
apply all `.gitignore` rules. One snapshot is bounded to 100,000 paths, 512 MiB
hashed, and 30 seconds.

This is not an artifact/CAS system. It cannot detect reads, transient changes,
or the author of a concurrent modification, and it cannot restore or roll back
the workspace.

## Trace journal is a separate contract

Tactus projects accepted frames and process facts into
`agenstro.trace/v1` records in `.tactus/runs/<run-id>/events.jsonl`, followed by
an atomically published `summary.json`. The trace API is not the plugin API:

- plugin frames describe one live request/response stream;
- trace records add local run IDs, sequence numbers, timestamps, and runtime
  event kinds; and
- the trace is factual evidence, not deterministic replay.

Run journals can contain provider output and have no built-in redaction. They
must not be committed or shared without review.

## Trust model

A configured command is arbitrary local code. It inherits the workspace,
environment, credentials, network, and user permissions. Version 1 has no
plugin signing, authentication, manifest trust, capability token, credential
broker, approval system, or sandbox. Process groups, bounds, strict JSON, and
argv execution are reliability mechanisms, not hostile-code isolation.
