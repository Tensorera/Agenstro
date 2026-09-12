# Timestamp-organized persistent task

This offline example separates `.tactus/segno.hs` scheduling/state organization
from `.tactus/scripts/010_business.hs`, whose only argument is its working
directory. Initialize the target workspace using `tactus init`, then
`segno init --root WORKSPACE --sdk /path/to/segno-flow`, and copy the two files
to those destinations. The service uses the standard Tactus runner, so `tactus`,
`cabal`, and GHC must be on its PATH.

```sh
segno service register daily-report --root /absolute/workspace --every-seconds 86400
segno service start --port 47832
segno service list --json
segno service status --json
segno service pause daily-report
segno service resume daily-report
```

Open `http://127.0.0.1:47832` for the read-only status page. A fresh registration
is due immediately. The service runs in the foreground; `segno.service` is an
optional systemd user-unit example. Adjust its installation paths, copy it to
`~/.config/systemd/user/segno.service`, then explicitly enable it with
`systemctl --user enable --now segno`. Automatic start without an interactive
login depends on the machine's user-session configuration. Other operating
systems can supervise the same foreground command with their service manager.

The machine registry defaults to the platform user data directory under
`agenstro/segno/registry.sqlite3`. `SEGNO_REGISTRY` or
`segno service --registry /absolute/registry.sqlite3 ...` selects another one.
The registry is lifecycle metadata; example business state remains in
`.tactus/segno-service-state/latest.json` and per-invocation work directories.

`Segno.Service.Protocol.serviceMain` parses exactly one ISO-8601 UTC timestamp
argument and atomically writes the following result to the unique
`SEGNO_SERVICE_RESULT_PATH` set by the service:

```json
{"api":"agenstro.segno-service-result/v1","timestamp":"2026-09-12T12:00:00Z","status_code":0,"info":{"message":"finished"}}
```

The timestamp must match the invocation. `status_code` is an integer in 0..255;
zero is success and nonzero is a known business failure. `info` is any JSON
value. The script must exit successfully after publishing either kind of known
result. Process failure, missing/malformed/oversized results, or mismatched
timestamps become `outcome_unknown` and pause the schedule. Results are limited
to 1 MiB, and process stdout/stderr are bounded separately. Human logs may use
stdout; they cannot be mistaken for a result.
The example propagates nested Tactus process failures as uncertainty. Return a
known nonzero business status only from an explicit business outcome, never
from an inference that a failed command made no external changes.

The service invokes each organizer as:

```sh
tactus run --root WORKSPACE --script WORKSPACE/.tactus/segno.hs --package segno-flow --timeout-seconds 1800 -- TIMESTAMP
```

Registration accepts `--task-timeout-seconds` (1..604800, default 1800).
The deadline applies to each Tactus build/run phase. Leases cover twice this
deadline plus 120 seconds. After a crash, the service waits for an existing
lease to expire, marks it unknown, and pauses that workspace. Graceful
interruption records uncertainty promptly. Inspect the work directory and
external effects before `resume`; resuming schedules a fresh future timestamp
and never replays the uncertain invocation.

Only one task may be registered per canonical workspace. Short SQLite
transactions prevent overlapping invocations across concurrent service/once
processes. This initial service runs tasks sequentially; a long task delays
other tasks but the web status listener remains responsive. Missed intervals
coalesce to the latest due timestamp. Known failures run again at the next
scheduled interval; they are not immediate retries. `pause` affects future
invocations and does not cancel work already running. `start --no-web` disables
the listener; `once` drains at most 100 due tasks without starting a listener.

Existing typed `segno install`, `segno driver`, and business-state CAS APIs
remain available separately. Do not register an organizer and a legacy driver
that independently launch the same external action. Invocation history is
retained in SQLite; this MVP does not prune history or implement exactly-once
external execution.
