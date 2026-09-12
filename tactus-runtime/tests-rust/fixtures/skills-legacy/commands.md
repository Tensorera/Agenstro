# Command reference

## Optional supervision and live output

Start `tactus watch --root /path/to/project --port 47831` before provider work
and open the complete printed URL to inspect live assistant output. Capture is
transient; recorded run status remains available separately.

`tactus run --daemon --daemon-provider NAME --script SCRIPT` enables periodic
monitoring through an independently registered provider. Automatic repair also
requires `--daemon-retry-script SCRIPT` or `[daemon].retry_scripts`; include
only entries whose effects can safely be repeated. A retry preserves original
arguments and reruns only the failed selected entry. `[daemon]` configures
`provider`, `interval_seconds` (60), `timeout_seconds` (120), `max_retries` (1)
and `retry_scripts` ([]). Unknown outcomes and exhausted budgets escalate to
stderr and the task's `daemon.jsonl` instead of rerunning.

For persistent scheduling, install the Haskell Segno package and initialize
its SDK linkage with `segno init --root PROJECT --sdk /path/to/segno-flow`.
Create `.tactus/segno.hs` using `Segno.Service.Protocol.serviceMain`, which
receives a UTC timestamp and publishes `(status_code, JSON info)` through the
service's result file. Let it organize work directories and call numbered
business scripts with their directory argument. Propagate uncertain child
outcomes as missing results rather than guessing a known business failure.

```sh
segno service register NAME --root PROJECT --every-seconds 3600
segno service start --port 47832
segno service status --json
```

The service registry spans workspaces. `segno service --registry PATH ...`
selects a separate registry; pause/resume controls future invocations and never
replays an unknown one. The status dashboard is read-only.

## Script and plugin commands

Pass paths and goals as separate quoted arguments. Tactus searches `--root`
and its parents for `.tactus/tactus.toml`; script paths are relative to the
resolved workspace root. Initialize a new workspace with `tactus init <path>`.

```sh
tactus list --root /path/to/project --json
tactus doctor --root /path/to/project --json
```

`check` takes positional sources, including helper modules. `run` takes
repeatable `--script` arguments and preserves their order; `--all` and ranges
use discovery order. Both require an explicit selection: paths, `--all`, or an inclusive
`--from` / `--through` range.

```sh
tactus check --root /path/to/project .tactus/scripts/010_main.hs .tactus/scripts/Support.hs
tactus run --root /path/to/project --script .tactus/scripts/010_main.hs
tactus run --root /path/to/project --script .tactus/scripts/010_main.hs -- 'workflow argument'
```

`list`, `check`, and `run` accept `--scripts-dir .tactus/<directory>` to select
one alternative source tree. Source arguments remain relative to the workspace
root, and Haskell module lookup follows the selected tree. Directories must
exist below `.tactus`; parent traversal and symlink paths are rejected. Without
this option, `--all` only selects `.tactus/scripts`, never Motivo methods.

```sh
tactus list --root /path/to/project --scripts-dir .tactus/motivoscript --json
tactus check --root /path/to/project --scripts-dir .tactus/motivoscript .tactus/motivoscript/020_investigate.hs
tactus run --root /path/to/project --scripts-dir .tactus/motivoscript --script .tactus/motivoscript/020_investigate.hs -- --input findings.md
```

Both accept `--timeout-seconds N` and repeated `--package NAME`. A timeout of
`0` disables the outer deadline. `--keep-going` continues after a failed entry;
it does not retry it or undo previous effects.

`tactus generate --root /path/to/project 'authoring goal'` invokes the configured
generation provider. It can type-check while authoring; generation does not
execute the resulting business workflow. Edits to helper modules count as
source changes. Select and run entries separately when the task needs execution.

For plugin authoring, read the workspace registry and the plugin's `describe`
response before constructing an invocation. `describe` reports methods and
capabilities; it does not validate task correctness.

```sh
tactus plugin-call --root /path/to/project --namespace effect project.tests describe
tactus plugin-call --root /path/to/project --namespace effect project.tests check --params '{"target":"parser"}'
```

For a custom executable plugin, Tactus sends one `agenstro.plugin/v1` JSONL
request with `id`, `method`, and `params`. Reply with a correlated terminal
`{"type":"result","id":"<request id>","ok":true,"value":...}` or
`{"type":"result","id":"<request id>","ok":false,"error":{"code":"...","message":"..."}}`.
Reserve stdout for protocol frames; use stderr for diagnostics. Check the
repository's plugin protocol reference for optional events and transport limits.
