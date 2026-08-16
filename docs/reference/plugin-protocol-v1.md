# Local plugin protocol v1

Clef providers and effects share one local, language-neutral process protocol.
Provider and effect registries remain separate even though their transport is
the same.

## Process contract

For each call the runtime:

1. starts the configured command in the workflow workspace;
2. inherits the caller environment, including provider credentials;
3. writes one UTF-8 JSON request followed by a newline to stdin;
4. closes stdin;
5. reads zero or more JSONL event records and exactly one terminal result from
   stdout; and
6. treats stderr as human diagnostics, never as protocol data.

Version 1 is deliberately one-shot. Process exit is its cancellation boundary;
there is no daemon, authentication handshake, socket discovery, or persistent
session hidden behind the core ABI.

The current Clef and Tactus clients preserve event order and buffer plugin
stdout until that one-shot process exits; this is not yet a live UI stream.
Clef also buffers plugin stderr, while Tactus inherits stderr so diagnostics
remain visible as they are written. The trusted-local core imposes no default
plugin deadline or output quota. A reference provider adapter honors an
explicit `options.timeout_seconds` for its direct CLI child, but this is not a
hard process-tree deadline; a descendant that retains a pipe can delay final
completion. An arbitrary unresponsive plugin can block its caller until it is
interrupted.

## Request

```json
{
  "api": "agenstro.plugin/v1",
  "id": "runtime-generated-correlation-id",
  "method": "invoke",
  "params": {}
}
```

`params` is an open JSON object owned by the selected plugin method. The core
must preserve unknown provider options rather than translating them into a
closed model or effort enumeration.

## Events and result

```json
{"type":"event","id":"...","event":{"type":"progress"}}
{"type":"result","id":"...","ok":true,"value":{}}
```

All stdout records must use the request `id`. Event subtypes are open: clients
preserve an unknown `event.type` and do not confuse it with a new top-level
frame kind. JSON object keys must be unique. Decimal or exponent numbers must
neither overflow nor underflow the finite IEEE-754 binary64 range; integral
JSON numbers retain arbitrary precision. Fractional values remain ordinary
host-language numbers, so exact decimal identifiers belong in JSON strings.
The same duplicate-key and numeric checks apply to Clef runtime configuration,
typed `jsonTask` results, and encoded outbound plugin requests. A successful
process produces one and only one result. Failure uses:

```json
{
  "type": "result",
  "id": "...",
  "ok": false,
  "error": {
    "code": "PROVIDER_EXITED",
    "message": "...",
    "details": {}
  }
}
```

An invalid record, a second result, data after the result, an ID/API mismatch,
or process exit without a result is a protocol failure. If the process may
have reached an external service before crashing, the runtime reports
`outcome_unknown`; it does not invent rollback or retry guarantees.

## Common methods

Every plugin supports:

- `describe`: return its name, kind, implementation version, operations, open
  option schema, and observed capabilities;
- `smoke`: with `live=false`, resolve the executable and obtain its version;
  with `live=true`, perform the plugin's documented minimal external call.

Provider plugins additionally support `invoke`:

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

They return a normalized object containing at least `text`, `exit_code`, and
provider metadata. A provider may return a session identifier, but independent
invocations are the default and session reuse must be explicit.

Effect plugins expose domain methods through `perform`. An observational
effect may also implement the controller lifecycle:

- `observe.begin {workspace, invocation}` -> arbitrary opaque JSON value;
- `observe.end {workspace, invocation, begin, outcome}` -> evidence, where
  `begin` is the unchanged value returned by `observe.begin`.

The runtime wraps provider invocations with configured controllers. Provider
output and effect evidence remain separate values.

The bundled path effect also exposes explicit opaque snapshots:

- `snapshot {workspace, options}` -> `{snapshot_id}`;
- `diff {workspace, before:{snapshot_id}, after:{snapshot_id}}` -> path lists;
- `forget {workspace, snapshot_id}` -> `{forgotten}`.

Callers cannot inspect or forge snapshot contents through this API. They should
`forget` snapshots that are no longer needed; observer state is cleaned after
`observe.end` even when the observed provider fails.

Observer cleanup is best-effort under process cancellation, not an exactly-once
transaction. Clef ends every begin value it has received, in reverse order, and
continues attempting later cleanup after one end fails. If an external
`observe.begin` completed but its terminal response was lost during
cancellation, Clef never obtained the opaque value and cannot synthesize a
matching end call; effects should make stale state forgettable or reclaimable.

## Bundled providers

The bundled adapters launch their native CLI in non-interactive, maximally
permissive mode:

- Codex: `codex exec --dangerously-bypass-approvals-and-sandbox --json ...`;
- Claude Code: `claude --print --dangerously-skip-permissions --output-format
  stream-json ...`;
- OpenCode: `opencode run --auto --format json ...` together with a local
  `permission = allow` override.

OpenCode's `--auto` only approves requests that would otherwise ask; an
explicit later `deny` can still win. Its smoke result therefore reports that
full bypass is not provable instead of claiming parity with the Codex and
Claude Code flags.

## `workspace.paths`

The first effect snapshots the workspace before a provider starts and reports
the final path delta afterward:

```json
{
  "added": [],
  "modified": [],
  "deleted": [],
  "type_changed": []
}
```

Each entry is a slash-separated workspace-relative path. Snapshots compare
file kind, byte size, and SHA-256 internally, but the public diff does not
publish file contents. The reference effect excludes `.git`, its own
`.tactus/path-effect` state, and Tactus' `.tactus/dist-newstyle` compiler cache.
The `.tactus` directory itself is a transparent container, so generated files
under `.tactus/scripts` and configuration changes remain observable. No other
`.gitignore` rules are applied.

This effect does not store content, publish artifacts, restore files, or infer
authorization. A final snapshot cannot observe reads or transient writes, and
changes made by concurrent processes are workspace deltas rather than proven
agent attribution.

## Trust model

A configured plugin command is arbitrary local code. This version intentionally
has no manifest signing, credential broker, policy token, or sandbox. The
runtime passes argument arrays without a shell, but that is process correctness,
not an authentication system.
