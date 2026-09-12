# Author a timestamp organizer

Inspect `.tactus/tactus.toml`, `.tactus/cabal.project`, and any existing
`.tactus/segno.hs` before editing. If this workspace is already scheduled,
prepare changes separately from its active sources, or pause future invocations
and check that in-flight work has finished before updating the live files.
Pausing alone does not stop an invocation already running.
For a new workspace, initialize Tactus and
then add Segno's SDK link:

```sh
tactus init /path/to/project --sdk /path/to/clef-sdk
segno init --root /path/to/project --sdk /path/to/segno-flow
```

The SDK directories contain `clef-sdk.cabal` and `segno-flow.cabal`, respectively.
Omit `--sdk` when installed SDK discovery already works. Both initializers
preserve existing workspace files; `segno init` alone does not supply the
task-specific organizer or register a schedule.

Copy or adapt [../assets/segno.hs](../assets/segno.hs) into
`.tactus/segno.hs` and [../assets/010_business.hs](../assets/010_business.hs)
into `.tactus/scripts/010_business.hs`. These files need no model and produce
a small placeholder report for each scheduled timestamp. Implement the requested
report inputs and content in the business script before enabling the real task.

## Contract

The service starts the organizer in the workspace root through:

```sh
tactus run --root /path/to/project --script .tactus/segno.hs \
  --package segno-flow --timeout-seconds 1800 -- 2026-09-12T12:00:00Z
```

This illustrates the service invocation, not a complete manual command: the
service also supplies a unique `SEGNO_SERVICE_RESULT_PATH`. `serviceMain`
requires that environment variable and an existing parent directory. Prefer
service execution when testing scheduling; an authorized manual diagnostic
run needs its own result path and does not create registry history.

Use `Segno.Service.Protocol.serviceMain :: (UTCTime -> IO (Int, Value)) -> IO ()`.
It parses exactly one ISO-8601 UTC timestamp, calls the organizer, validates
the result, and atomically publishes it to the result path. The timestamp is
the logical scheduled time, not necessarily the wall-clock start time.
The resulting JSON has this shape:

```json
{"api":"agenstro.segno-service-result/v1","timestamp":"2026-09-12T12:00:00Z","status_code":0,"info":{"message":"finished"}}
```

`status_code` is an integer in `0..255`: zero means success; nonzero means a
known business failure. In either case the organizer must exit normally after
publishing its result. `info` may be any JSON value; keep it concise with
artifact references rather than full reports. The complete result is limited
to 1 MiB and its timestamp must match the invocation. stdout is for diagnostics,
not the result channel. Use the helper instead of constructing result JSON by
hand.

An exception, process failure, malformed or missing result, or timestamp
mismatch becomes `outcome_unknown` and pauses the schedule. Do not catch a
subprocess failure merely to translate its exit code into a known business
status. The subprocess may have changed external state before failing. A
known failure needs explicit business evidence, such as a validated terminal
response with defined semantics.

## State and business work

The template derives a work directory from the UTC timestamp, passes it as
the business script's positional argument after `--`, and updates
`.tactus/segno-service-state/latest.json` after successful completion. Business
scripts can be tested independently against a chosen directory. Keep their
argument convention consistent; a positional directory and `--work-dir PATH`
are both reasonable if the scripts actually parse the chosen form.

For the bundled business script, an authorized standalone run is:

```sh
tactus run --root /path/to/project --script .tactus/scripts/010_business.hs \
  -- /chosen/work-directory
```

The organizer owns directory naming and completion markers; the business
script owns its work and artifacts. Publish completion state only after the
required outcomes have been established. For non-idempotent actions, preserve
external correlation IDs and check authoritative state before any retry.
Do not replay old timestamps just because a directory already exists.

The template's nested `tactus run` uses `callProcess`: a failed subprocess
throws and leaves Segno to record uncertainty. It does not infer that the
business operation had no effect. The organizer is trusted Haskell with
ordinary IO access; source selection is not a sandbox.

## Validate and execute

Select the standard organizer explicitly; it is excluded from default
`run --all`. It cannot be replaced by an arbitrary unnumbered runnable entry.
For this template, compile both files without executing their business code:

```sh
tactus check --root /path/to/project --package segno-flow .tactus/segno.hs
tactus check --root /path/to/project .tactus/scripts/010_business.hs
```

When task execution is authorized, use the selected registry and the commands
in [service.md](service.md). For a model-free trial of the bundled templates,
an isolated registry avoids enabling the trial in a running machine service:

```sh
segno service --registry /path/to/trial/registry.sqlite3 register trial-report \
  --root /path/to/project --every-seconds 86400
segno service --registry /path/to/trial/registry.sqlite3 once
segno service --registry /path/to/trial/registry.sqlite3 status --json
```

Use a separate workspace as well if the project already has an active job:
overlap prevention applies within a registry, not across independent registries.
Verify a `succeeded` invocation, its `info`, the occurrence's `report.txt`, and
the organizer's `latest.json`. This proves the template path, not the validity
or retry safety of a subsequently added business operation.
