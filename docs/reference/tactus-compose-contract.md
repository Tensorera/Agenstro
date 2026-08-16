# Tactus compose contract

This contract belongs to the compatibility `tactus compose --agent` and
`tactus loop` commands. It is not the production Studio composition path.
Production Studio lets a native terminal agent edit
`.tactus/main_script.py` directly and therefore expects no agent response
object.

On the compatibility path, Codex, Claude Code, and OpenCode must return one
object matching Tactus's local JSON Schema. The same validation applies to
all supported agents.

## Compose input context

Every agent compose is stateless. Tactus rebuilds a version 2 JSON context
from SQLite and the exact project workspace for each request. The context includes
the objective, guarded current attempt, allowed actions, one current
`additional_instruction`, workspace inventory, bounded cell history, complete
workflow memory, and recent durable interaction history.

The interaction portion has this shape:

```json
{
  "interaction": {
    "mode": "compose",
    "messages": [
      {
        "message_id": "stable-message-id",
        "role": "user",
        "content": "Validate the result against the paper's reported range.",
        "mode": "discussion",
        "created_at": "2026-07-29T12:00:00Z",
        "in_reply_to": null
      }
    ],
    "messages_omitted_count": 0
  }
}
```

| Field | Type | Constraint |
| --- | --- | --- |
| `interaction.mode` | string | Current mode; agent compose is accepted only in `compose` mode |
| `messages` | object array | Newest durable messages in chronological order |
| `messages[].message_id` | string | Stable journal message identifier |
| `messages[].role` | string | `user`, `assistant`, or `system` |
| `messages[].content` | string | Bounded message content supplied to the agent |
| `messages[].mode` | string | Mode in which the message was written: `compose` or `discussion` |
| `messages[].created_at` | string | Durable creation timestamp |
| `messages[].in_reply_to` | string or null | User message referenced by an assistant reply |
| `messages_omitted_count` | integer | Older or budget-pruned messages not included in this request |

Tactus initially selects the newest 50 messages and clips each message to
4,000 characters for context. The complete compose context has a 160,000
character serialized budget. If other mandatory context consumes that budget,
Tactus drops older interaction messages, increments
`messages_omitted_count`, and may clip retained content further. These limits
do not truncate the original messages in SQLite.

`tactus message` appends Compose-mode user steering without invoking an
agent. A Compose-mode message increments the workflow revision, so a compose
decision already in flight cannot commit after new human steering arrives.
Discussion user messages and assistant replies remain in interaction history
after `discussion-end` and therefore enter the next compose context. The
composer must treat later messages as able to supersede earlier advice.

Interaction history is steering, not a replacement for workflow memory or
verified artifacts. Cross-cell facts still belong in `workflow_memory`,
project files, `.tactus/helpers`, or an explicit database.

## Root object

| Field | Type | Constraint |
| --- | --- | --- |
| `action` | string | `run` or `finish` |
| `summary` | string | 1–2000 characters |
| `source` | string | Self-contained Python for `run`; empty for `finish` |
| `memory` | object | Complete bounded workflow memory |

Additional root fields are rejected.

## Workflow memory

| Field | Type | Limit |
| --- | --- | --- |
| `current_state` | string | 6000 characters |
| `decisions` | string array | 20 items; 500 characters each |
| `open_issues` | string array | 20 items; 500 characters each |
| `artifacts` | object array | 30 items |
| `validation` | string array | 20 items; 500 characters each |

Each artifact requires:

| Field | Type | Constraint |
| --- | --- | --- |
| `path` | string | 1–500 characters |
| `description` | string | 1–500 characters |
| `status` | string | `planned`, `created`, `verified`, or `missing` |

The agent returns the complete updated memory on every request. Tactus stores
the object with the compose decision or workflow completion. Cross-cell data
that must be verified belongs in project files, helper modules, or an explicit
database; Python process memory is not durable.

## Action rules

- `run` creates a draft cell for the guarded phase and attempt.
- A failed attempt can only be repaired by a new cell in the same phase.
- `finish` is valid only when the objective is satisfied.
- `finish` completes the runtime root and must use an empty `source`.
- The cell runs once from the exact project root in a fresh Jupyter kernel.

Current in-place projects use the project directory directly. The compatibility
compose contract does not imply a temporary or detached workspace. If one cell
needs isolation, the composer must put explicit isolation and publication
steps in its source. Legacy detached runtime roots execute the same contract
in their configured compatibility workspace.

The canonical machine-readable schema is packaged at
`tactus_runtime/schemas/compose-output.schema.json`.
