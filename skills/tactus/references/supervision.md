# Daemon supervision and live output

## Enable a daemon for one run

Daemon monitors selected entries through periodic prompts to a registered agent
provider. It is attached to `tactus run`, not a machine scheduler; use the sibling
[Segno skill](../../segno/SKILL.md) for persistent schedules. Configuring
`[daemon]` alone does not enable it. Start with monitoring and local reporting:

```sh
tactus run --root /path/to/project --script .tactus/scripts/010_work.hs \
  --daemon --daemon-provider reviewer -- /path/to/work-directory
```

Register `reviewer` in `.tactus/tactus.toml` independently of the business
provider. It may use a different adapter, native executable, model, effort, and
provider options. Reuse a working provider configuration and change only what
the task needs; do not assume that a provider's accepted model or effort strings
are shared by another provider.

```toml
[providers.reviewer]
command = ["tactus", "provider-host", "claude-code"]
# Optional: choose values supported by this installed native CLI.
# model = "provider-specific-model"
# effort = "high"

[daemon]
provider = "reviewer"
interval_seconds = 60
timeout_seconds = 120
max_retries = 1
retry_scripts = []
```

The selected native CLI owns authentication. Built-in adapters request
autonomous tool execution; the daemon retains the authority of that adapter
and the native CLI's effective policy.
If executable discovery is ambiguous, set `executable` under
`[providers.reviewer.options]` to the intended native CLI path for this machine.
Daemon requests can incur provider usage even when the business script does not
invoke a model.

| Workspace setting | Default | Constraint / CLI override |
| --- | --- | --- |
| `provider` | Workspace `default_provider` | Registered key; `--daemon-provider NAME` |
| `interval_seconds` | `60` | `1..86400`; `--daemon-interval-seconds N` |
| `timeout_seconds` | `120` | `61..3600`, including provider cleanup; config only |
| `max_retries` | `1` per entry | `0..10`; `--daemon-max-retries N` |
| `retry_scripts` | `[]` | Workspace-relative paths; repeat `--daemon-retry-script PATH` to add entries |

All CLI overrides require `--daemon`. The interval is the delay between
completed monitoring requests, not a promise of exact wall-clock ticks. The
script's `--timeout-seconds` controls its own execution deadline, separately
from the daemon provider deadline.

## Authorize bounded repair

An entry must be explicitly retryable before daemon may edit and rerun it. Use
the workspace `retry_scripts` list or a run-local addition:

```sh
tactus run --root /path/to/project --script .tactus/scripts/010_work.hs \
  --daemon --daemon-provider reviewer \
  --daemon-retry-script .tactus/scripts/010_work.hs \
  --daemon-max-retries 1 -- /path/to/work-directory
```

Choose entries whose repeated effects are safe, such as deterministic output
replacement or operations with stable idempotency keys. A known process failure
can follow partial external work; do not infer repeatability from a nonzero exit
alone. Keep non-repeatable sends, payments, and destructive changes outside the
retry allowlist unless the actual workflow makes repetition safe. `max_retries = 0`
disables automatic repair while retaining monitoring.

Monitoring accepts a provider decision to continue or escalate. After a known
failure, a repair decision may propose one exact textual replacement in the
failed entry. Tactus requires a unique, nonempty old fragment, at most 8 KiB per
fragment, an entry at most 128 KiB, and unchanged source since the proposal was
requested. It atomically applies the change and reruns only that selected entry
with its original arguments. It does not repair helper files or roll back prior
effects. Inspect the resulting source diff and verify the business outcome.

`OutcomeUnknown`, timeouts, cancellation, invalid proposals, changes outside this
boundary, unavailable reporting, and exhausted budgets escalate without an
automatic rerun. Once the task escalates, automatic repair remains disabled for
that task. Reconcile artifacts and external effects before making another
attempt. Provider prompts ask for a proposal without tool use, but these rules
do not sandbox the native provider's existing authority.

Inspect `daemon.jsonl` in the enclosing task run directory for monitoring,
attempts, patch metadata, and escalation; escalations also appear on stderr.
Patch records store hashes and sizes, not the full old/new source. A bounded
stderr tail and source can be sent to the repair provider. Reporting is local;
no email, chat, or webhook delivery is configured by daemon. Use
[outcomes.md](outcomes.md) for run inspection commands and interpretation.

## View live provider output

Start the viewer before provider calls in the same workspace:

```sh
tactus watch --root /path/to/project --port 47831
```

Open the complete printed URL, including `#token=...`. In another terminal,
launch the selected run, generation, or provider invocation. The viewer binds
to `127.0.0.1`, is read-only, and uses a temporary `.tactus/watch.json` connection
descriptor. Only one viewer owns a workspace at a time. Treat the complete URL
and descriptor as local access credentials; the page may show sensitive model
output.

Select a run to filter its live events and inspect recorded run history. The
viewer retains at most 4 MiB / 4096 live events in memory. A page refresh can
reconnect to retained output; stopping the viewer discards it. Start calls after
the viewer so adapters discover the channel. Packet loss, truncation, and
expired history are shown explicitly. A closed or slow viewer does not change
the provider's terminal result.

| Built-in adapter | Observable text granularity |
| --- | --- |
| Claude Code | Incremental partial-message text deltas |
| OpenCode | Native text events |
| Codex CLI | Native JSON message events, which may contain completed messages |

Custom provider hosts do not acquire live text automatically. The viewer shows
recognized assistant text; it does not reconstruct missing token deltas. Raw
stream text is not persisted as a transcript in `events.jsonl` or `summary.json`;
durable run status and journal evidence remain available separately. Motivo's
HTML reports are offline snapshots, and Segno's dashboard shows durable
scheduling state.
