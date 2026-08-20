# Session document and control API v1

`agenstro.session/v1` is the durable projection of one human decision session.
Tactus owns its storage; Motivo and other clients use the commands on this page
and never parse `.tactus/sessions` directly.

This stage defines read and answer control. Planner execution and
`session advance` are reserved until the planner registration contract is
decided.

## Storage and durability

The runtime-internal layout is:

```text
.tactus/
  sessions/
    session-7f3a91/
      session.json
      session.lock
      transcript.jsonl
```

`session.json` is replaced atomically. Answer updates use a per-session
cross-process lock with a bounded wait, re-read current state while holding the
lock, then compare the supplied turn. Transcript records are append-only audit
evidence and include the answered brief snapshot. An interrupted append is
repaired to the last complete record under the lock, and a retry with the same
stable `answerId` does not append duplicate evidence. Clients must not depend
on filenames or mutate this layout.

Static symlinks, hard-linked auxiliary files, and Windows reparse-point
directories are rejected. This is reliability hardening for a trusted local
workspace, not protection from a same-user hostile process racing directory
replacement between filesystem operations. Stop other workspace writers while
operating or backing up sessions; handle-relative cross-platform storage is a
separate security design.

An older initialized workspace need not already contain `sessions/`; listing
such a workspace returns an empty collection. New `tactus init` operations
create the directory.

## Session view

```json
{
  "api": "agenstro.session/v1",
  "sessionId": "session-7f3a91",
  "label": "Desk build",
  "state": "awaiting_answer",
  "turn": "3",
  "pending": {
    "api": "agenstro.session/v1",
    "sessionId": "session-7f3a91",
    "turn": "3",
    "findings": [
      {
        "summary": "A thick solid top exceeds the surveyed lift ratings.",
        "source": "corpus: 40 commercial frames"
      }
    ],
    "question": {
      "axis": "desk.frame",
      "prompt": "Should the desk height be adjustable?",
      "options": [
        {
          "id": "sit-stand",
          "label": "Motorised sit/stand",
          "coordinates": {"height": "adjustable", "cost": "high"}
        },
        {
          "id": "fixed",
          "label": "Fixed built frame",
          "coordinates": {"height": "fixed", "cost": "low"}
        }
      ],
      "reversibility": "irreversible",
      "dependsOn": []
    },
    "stakes": [
      {
        "option": "sit-stand",
        "effect": "Raises the frame cost and reduces the top thickness budget.",
        "reversibility": "irreversible"
      }
    ],
    "remainingSurface": ["desk.frame", "desk.finish"],
    "remainingFloor": ["desk.frame"]
  },
  "answered": [],
  "startedUnixMs": "1787200000000",
  "updatedUnixMs": "1787200100000"
}
```

`state` is the closed set `planning`, `awaiting_answer`, `delivered`, and
`abandoned`. `pending` is present exactly for `awaiting_answer`. Its session
identity and turn must match its parent.

A question contains two to six uniquely identified options. `defaultOption`
is optional and, when present, names one of them. Stakes may name only current
options, and one option may have multiple consequences. Option ids are stable
tags matching `[A-Za-z0-9][A-Za-z0-9._-]*`. `remainingFloor` is a subset of `remainingSurface`; the former is what
must still be decided, while the latter also includes conditional branches.

Counts and timestamps are decimal strings so JavaScript does not truncate
64-bit values. Coordinates and axis identities are open. Reversibility is the
closed set `reversible`, `costly`, and `irreversible`.

## Commands

```powershell
tactus session list --root ROOT --limit 50
tactus session show --root ROOT --session session-7f3a91
tactus session answer --root ROOT --session session-7f3a91 `
  --turn 3 --axis desk.frame --option fixed --note "Prefer repairable joinery"
```

Each command writes exactly one `tactus.control/v1` envelope. A list succeeds
with:

```json
{
  "api": "tactus.control/v1",
  "command": "session.list",
  "status": "completed",
  "data": {"api": "agenstro.session/v1", "sessions": []}
}
```

`show` and `answer` return one session view as `data`.

## Answer semantics

An answer is a compare-and-set operation, not a blind update. Tactus verifies
all of the following while holding the session lock:

1. the current state is `awaiting_answer`;
2. the supplied turn equals the current turn;
3. the axis equals the pending question axis; and
4. the option belongs to the pending question.

On success, an earlier answer for the same axis is replaced, the new answer is
appended to the right-biased projection, `defaulted` is `false`, `pending` is
removed, and state becomes `planning`. The optional note is transcript
evidence and does not become an unvalidated field in the current view.

A stale turn returns an error envelope with code `session_turn_stale`. A UI
should refetch and show the current question instead of applying the old
choice or retrying automatically.

Other stable codes are `session_invalid_id`, `session_invalid_argument`, `session_not_found`,
`session_corrupt`, `session_axis_mismatch`, `session_option_invalid`,
`session_state_invalid`, and `session_io_failed`. Error messages are bounded
and do not expose the workspace root.

Each session document is limited to 1 MiB and each transcript record to 2 MiB.
Listing scans at most 2,000 entries, reads and serializes at most 8 MiB of
recognized session data, returns at most 200 sessions, and sorts them by newest
update before applying the requested limit. The 8 MiB projection ceiling leaves
headroom inside Motivo's 9 MiB control-stdout budget.

## Compatibility

- Reject an unknown top-level `api` and unknown closed enum values.
- External clients may tolerate additive fields in a recognized v1 document,
  but must project it into their own bounded, strict IPC shape.
- Session and axis identities are stable. Renaming an axis discards its stored
  answer relationship.
- Session ids match `session-[A-Za-z0-9-]+`, are bounded to 128 characters,
  and are matched with exact case. Producers should prefer lowercase and must
  not create identities that differ only by case across workspaces.
- The renderer never receives a workspace path and never writes session files.
- Motivo IPC additionally binds list/show/answer requests to the opaque
  workspace handle that produced the visible view, preventing a stale choice
  from being redirected into a newly opened workspace.

See [ADR-0006](../adr/0006-motivo-session-pattern.md) for the staged planner
boundary and the [implementation status](../design-bundle-status.md) for
deferred decisions.
