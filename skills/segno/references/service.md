# Operate the machine-local service

Use the same registry for inspection, registration, scheduling, and recovery.
The default is `agenstro/segno/registry.sqlite3` below the platform's user-data
directory. `SEGNO_REGISTRY` overrides it; an explicit CLI `--registry PATH`
overrides the environment. The registry option belongs before the service
subcommand:

```sh
segno service --registry /path/to/registry.sqlite3 list --json
segno service --registry /path/to/registry.sqlite3 status --json --limit 20
```

Inspect the existing job and history first. The registry contains lifecycle
metadata; business files stay in each workspace. Back up or inspect it without
manually rewriting leases or terminal outcomes.

## Registration and timing

Once the organizer exists and selected sources type-check, register the task
when scheduled execution is in scope:

```sh
segno service register daily-report --root /path/to/project \
  --every-seconds 86400 --task-timeout-seconds 1800
segno service list --json
segno service status --json --limit 20
```

The first invocation is immediately due; an already running service can pick
it up as soon as registration completes. There is one registered task per
canonical workspace, with a unique name in the registry. Registration is not
an update operation for an existing job; do not work around that by directly
editing registry rows. Inspect `segno service --help` before assuming rename,
interval-update, or unregister commands exist.

Intervals are `1..31536000` seconds, not wall-clock cron schedules. Missed
intervals coalesce to the latest due timestamp. Tasks execute sequentially,
so a long task delays other work. `once` handles at most 100 due tasks and
exits; `start` keeps polling. Claims prevent overlapping workspace invocations
across service/once processes using the same registry. Avoid scheduling the
same external action through separate registries or the legacy `segno driver`.

`--task-timeout-seconds` accepts `1..604800`, default `1800`, and applies
separately to Tactus's build and run phases. A cold Cabal build may need a
larger deadline. Lease duration is twice this deadline plus 120 seconds.
A known business failure is recorded and the next normal interval remains
eligible; this does not trigger an immediate retry.

## Start and observe

Inspect the machine's existing supervisor and listener before starting another
process. On Linux with a Segno user unit, useful checks are:

```sh
systemctl --user status segno
systemctl --user cat segno
journalctl --user -u segno -n 50 --no-pager
```

An active unit should be managed through that unit. Confirm its executable,
registry environment, and PATH include the intended `segno`, `tactus`, Cabal,
and GHC. Do not start a second foreground server merely to open the dashboard.
If there is no managed process and foreground execution is requested:

```sh
segno service start --port 47832
```

It runs until interrupted and serves a read-only dashboard at
`http://127.0.0.1:47832`; `GET /api/status` exposes the same status as JSON.
The web listener remains responsive while a task runs. Ports must be in
`1024..65535`. `start --no-web` runs only the scheduler. The dashboard cannot
register, pause, resume, or execute jobs; use the CLI for those actions.

An OS startup service is optional and separate from registering business
tasks. When deployment includes automatic startup, configure the existing
service manager with the foreground command and the correct environment.
On a systemd user installation, `systemctl --user enable --now segno`
starts an installed unit and enables startup for the user session. Do not
assume this means running before login; that depends on host configuration.

Use `tactus watch --root /path/to/project --port 47831` in the workspace for
provider output during service-launched tasks; open its printed complete URL
including the token fragment. Segno's dashboard shows schedules and outcomes,
not the provider transcript. Providers must be configured in the workspace.
Daemon supervision, if desired, belongs on the organizer's nested business
`tactus run`, with bounded retries only for explicitly selected safe scripts.

## Pause and recover

```sh
segno service pause daily-report
segno service status --json --limit 20
segno service resume daily-report
```

Pause prevents future invocations and does not cancel one already running.
Run `resume` only when the requested task includes recovery and the uncertain
effects have been reconciled. It schedules a new future timestamp, preserves
the old outcome, and never replays that invocation.

When status shows `outcome_unknown`, inspect the run's diagnostic details,
workspace artifacts, organizer state, and authoritative external effects.
Resolve whether work completed, needs compensation, or needs a separately
authorized retry. Fix the underlying issue before resuming; changing an
unknown result into a known failure by assumption can duplicate effects.

A crash can leave a `running` lease. The scheduler waits for expiry, records
uncertainty, and pauses that workspace; graceful interruption records it
promptly. Do not launch a parallel replacement to bypass the lease. Keep the
service running to recover expired leases, or use `once` when executing other
due work in that registry is also authorized. History persists across restarts;
the service does not promise exactly-once external execution or prune history.
