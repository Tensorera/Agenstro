---
title: Segno trigger and state plugin wire v1
status: alpha
last_verified: 2026-08-16
applies_to: "Segno Flow Haskell 0.3.0.0"
---

# Segno trigger and state plugin wire v1

This page defines the language-neutral payloads used by Segno trigger and
business-state plugins. Every call is one `agenstro.plugin/v1` JSONL request
and exactly one terminal result as defined by the [local plugin
protocol](plugin-protocol-v1.md). The schemas below describe `params` and the
successful terminal `value`; plugin failures use the standard structured
`error` object.

All timestamps are UTC RFC 3339 values. Backend revisions and cursors are
opaque JSON values unless a method below narrows their type. Plugins must
accept unknown fields inside open `config`, `payload`, `cursor`, and state
`value` documents. The listed control fields have fixed meanings; v1 receivers
may ignore additional control fields for forward compatibility, and senders
must not rely on an unrecognized field changing the operation's semantics.

## Trigger plugin

A trigger plugin calculates occurrences; it never sleeps. The Segno driver
owns waiting, durable cursor advancement, duplicate suppression, and task
lifecycle.

### `describe` and `smoke`

`describe` and `smoke` use empty/open params. A trigger `describe` result
should list `plan`, `poll`, `acknowledge`, and `smoke`, and should report that
the plugin does not wait. `smoke` must remain offline unless its own open
options explicitly request a live probe.

### `plan` / `poll`

Version one gives `plan` and `poll` the same pure planning contract. The
driver calls `poll`; `plan` exists for inspection and deterministic tests.

```json
{
  "workspace": "D:/work/project",
  "source_id": "each-minute",
  "config": {"every_ms": "60000"},
  "cursor": {"logical_time": "2026-08-16T12:00:00Z"},
  "now": "2026-08-16T12:03:00Z",
  "limit": 100
}
```

`cursor` is `null` before the first acknowledged occurrence. `limit` is an
integer from 1 through 1000. For identical inputs the plugin must return the
same ordered occurrences and next wake:

```json
{
  "occurrences": [
    {
      "logical_time": "2026-08-16T12:01:00Z",
      "cursor": {"logical_time": "2026-08-16T12:01:00Z"},
      "idempotency_key": "each-minute:20260816T120100Z",
      "payload": {"logical_time": "2026-08-16T12:01:00Z"}
    }
  ],
  "next_wake": "2026-08-16T12:02:00Z"
}
```

Occurrences must be in increasing logical-time order. An idempotency key must
be stable for the same logical occurrence, contain 1–512 Unicode scalar values,
and contain no C0 or DEL control character. `next_wake` may be `null` for a
source with no planned wake. When fewer than `limit` occurrences are returned,
a non-null `next_wake` must be later than the request's `now`; a full page may
use an earlier wake to request immediate bounded catch-up. Returning more than
`limit` occurrences is a protocol failure. The driver independently enforces a
one-second minimum sleep so stale persisted wake data cannot create a hot loop.

Segno first inserts every occurrence into lifecycle storage. Only after that
durable insert does it call `acknowledge`; only after all acknowledgements
succeed does it persist the latest returned cursor. A crash may therefore
repeat `poll` and `acknowledge`. Both operations must be idempotent.

### `acknowledge`

```json
{
  "workspace": "D:/work/project",
  "source_id": "each-minute",
  "occurrence_id": "occ:…",
  "idempotency_key": "each-minute:20260816T120100Z",
  "cursor": {"logical_time": "2026-08-16T12:01:00Z"}
}
```

Success returns:

```json
{"acknowledged": true}
```

An acknowledgement must not itself advance Segno's cursor; the driver remains
the cursor authority.

## Business-state plugin

Business state is separate from Segno's private lifecycle store. A workflow
can read and change its typed state but cannot set `Running`, alter attempts,
or mint fencing tokens.

### `load`

```json
{
  "workspace": "D:/work/project",
  "state_key": "example.active-window",
  "schema_version": 1,
  "initial": {"capturedWindows": 0}
}
```

The backend creates the initial revision if the key is absent, then returns a
snapshot:

```json
{
  "key": "example.active-window",
  "revision": "7",
  "schema_version": 1,
  "value": {"capturedWindows": 7}
}
```

`revision` is either `null` or a non-empty opaque string. The same revision
must identify the same stored value.

### `compare-and-set`

```json
{
  "workspace": "D:/work/project",
  "state_key": "example.active-window",
  "expected_revision": "7",
  "schema_version": 1,
  "value": {"capturedWindows": 8},
  "conflict": "compare-and-set",
  "operation_id": "capture-active-window",
  "occurrence_id": "occ:…",
  "fencing_token": "opaque-driver-token",
  "fencing_epoch": 1
}
```

An applied write returns exactly the new revision:

```json
{"applied": true, "revision": "8"}
```

A definite revision or fence conflict returns:

```json
{"applied": false, "current_revision": "9"}
```

`current_revision` may be `null` when no state exists. The tuple
`(state_key, occurrence_id, operation_id)` is an idempotency key: replaying an
identical request must return its original applied revision, while reusing the
tuple with a different request must be rejected.

`fencing_epoch` is the positive, monotonically increasing attempt number for
this occurrence. The revision comparison, write, operation record, and update
of the backend's highest accepted `(fencing_epoch, fencing_token)` for
`(state_key, occurrence_id)` must form one backend transaction. A lower epoch,
or the same epoch paired with another token, is a definite conflict. This rule
lets PostgreSQL, Redis, and other state plugins reject an older attempt without
reading Segno's private lifecycle database. The built-in SQLite backend adds a
stronger same-transaction check that the occurrence is currently `Running`
under the supplied epoch and token. A late first write can still be externally
visible after a transport loss; Segno therefore reports that window as
`OutcomeUnknown` and does not claim cross-backend exactly-once semantics.

A transport loss after sending this method is `OutcomeUnknown`, not a safe
automatic retry signal. The operation id makes an explicit retry inspectable
and idempotent, but does not make external workflow effects exactly once.

### `append`

`append` records a business event without replacing the current state:

```json
{
  "workspace": "D:/work/project",
  "state_key": "example.active-window",
  "event_kind": "observed",
  "value": {"count": 8},
  "occurrence_id": "occ:…"
}
```

Success is `{"appended": true}`. `occurrence_id` may be `null`. Append is an
audit/event capability and is not a substitute for fenced compare-and-set.

### `history`

```json
{
  "workspace": "D:/work/project",
  "state_key": "example.active-window",
  "limit": 100
}
```

`state_key` may be omitted to query all keys; `limit` is 1 through 1000. The
result is `{"entries": [...]}` in newest-first order. Entry payloads are open
for forward-compatible backend metadata.

## Failure and conformance rules

- A plugin must keep stdout exclusively for UTF-8 JSONL protocol frames;
  diagnostics go to stderr.
- Every request receives one correlated terminal result. A missing or invalid
  result is not converted into a business failure.
- Trigger cursor and acknowledgement behavior is at least once.
- State values and trigger payloads are open JSON, while methods and lifecycle
  semantics remain versioned control data.
- No plugin may claim exactly-once workflow or external-effect execution.
- Contract tests should cover duplicate poll/ack, catch-up limits, stale
  fences, repeated operation ids, CAS conflicts, invalid UTF-8/JSON, and a
  transport failure after a state request was sent.
