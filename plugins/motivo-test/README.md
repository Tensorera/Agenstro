# Motivo experiment effect

`motivo-test` implements the one-shot `agenstro.plugin/v1` protocol. It is a
project effect used by Motivo Haskell methods, not a provider or a task runner.
The first execution backend supports **Linux with Bubblewrap 0.8 or newer at
`/usr/bin/bwrap`**. The required `--disable-userns` capability was added in
[Bubblewrap 0.8](https://github.com/containers/bubblewrap/releases/tag/v0.8.0).
Other
platforms, missing Bubblewrap, and unavailable namespace/mount features fail
without running the experiment. No model or network request is made by this
plugin.

Build and install from the repository:

```sh
cargo install --path plugins/motivo-test --locked
```

Register in the initialized project's `.tactus/tactus.toml`:

```toml
[effects."motivo.test"]
command = ["motivo-test"]
```

The plugin's working directory must be the initialized project root. The effect
uses that directory, not an input-supplied workspace path.

## Methods

- `describe`: returns implementation, input bounds, backend and directory rules.
- `smoke` with `live=false` (or omitted): checks backend presence without starting
  an experiment. `execution_verified=false` is deliberate: only a real `run`
  proves this environment permits the required namespaces.
- `run`: executes one experiment and returns its factual outcome.

Example request:

```json
{"api":"agenstro.plugin/v1","id":"probe-1","method":"run","params":{"run_id":"20260905-investigation","sample_id":"small-input","argv":["/bin/sh","-c","cp /project/test/fixture.txt input.txt; python3 /project/tools/probe.py input.txt"],"timeout_seconds":30}}
```

`argv` is an argument array, never implicit shell input. Use an explicit shell
only when the experiment needs one. The command runs in `/work` and reads the
project through `/project`. `run_id` and `sample_id` each contain 1–100 ASCII
letters, digits, underscores or hyphens. `argv` contains 1–256 strings. The
request limit is 1 MiB. `options` may be absent or `{}`; unknown parameters and
options are rejected.

Tactus may also inject `workspace`. When present, it must resolve to the actual
plugin working directory; it cannot redirect writes or select another project.

`timeout_seconds` is an integer from **1 through 600**, default **600**. One
monotonic budget covers preparing mounts, inspecting fixtures, compilation
inside the experiment command, and every experiment subprocess. The supervisor
reserves up to 550 ms **within** that budget for termination and polling, so a
one-second request permits less than one second of payload work. Saving the
terminal record after termination does not extend the experiment's execution.
No automatic retries or deadline resets occur between commands.

## Filesystem and process boundary

The only persistent writable directory available to the experiment is:

```text
.tactus/motivotest/<run_id>/<sample_id>/
```

It is mounted as `/work`; the entire project is separately read-only at
`/project`. `/tmp`, `HOME`, and writable cache/configuration locations point
inside `/work`. The sandbox root and its separate `/proc` and `/dev` mounts are
read-only; minimal devices such as `/dev/null` are available. Logs are protected
from payload writes by a read-only mount at `/work/.motivo-logs` while the trusted
host worker drains output through already-open file descriptors.

System tools under `/usr` and its normal `/bin`, `/lib`, etc. layout are
read-only. Existing mise **installation** and rustup **toolchain** directories
can be mounted read-only; PATH preserves only directories in those roots or
`/usr`. General user homes, provider configuration, SSH credentials, shell
startup files, inherited environment variables, host runtime directories and
host sockets are not supplied. Project files remain readable, including any
secrets that the project itself contains. This is not a data-redaction tool.
Project Unix sockets, FIFOs and device files are masked before execution.

Network and IPC namespaces are separate. Nested user namespaces are disabled,
capabilities are dropped, and a private PID namespace contains all descendants.
Bubblewrap's parent-death handling ties that namespace to its worker. The worker
also arms Linux `PDEATHSIG` and verifies its parent identity before reading the
request: killing the public plugin, even with SIGKILL, destroys its experiments.
This containment covers descendants that call `setsid`, unlike a process group
alone. The existing Tactus process supervisor owns the deadline and cancellation;
there is no daemon or separately persistent session.

Fixtures may be prepared in a sample directory before invocation. Every path
component must be a real directory; sample fixtures must be regular files with
one hard link or real directories. Symlinks, hard-linked files, special files,
and nested host mounts are rejected before execution. The experiment may create
links within its confined filesystem, but cannot use them to write host files
outside `/work`. Host-side agents should not concurrently replace the sample
directory or its ancestors while an experiment is being prepared or running.

Each sample executes once. The `.motivo-logs` directory is an execution marker;
reuse is rejected so a retry cannot silently overwrite earlier evidence. Choose
a new sample ID after any attempted execution, including startup failure.
Logs keep at most **1 MiB each**; excess output is drained and discarded. This
does not impose a general disk, CPU or RAM quota on payload-created artifacts.

## Result semantics

A successful plugin operation returns:

```json
{
  "status": "exited",
  "exit_code": 0,
  "signal": null,
  "duration_ms": 35,
  "timeout_seconds": 30,
  "workspace_dir": ".tactus/motivotest/20260905-investigation/small-input",
  "stdout_path": ".tactus/motivotest/20260905-investigation/small-input/.motivo-logs/stdout.log",
  "stderr_path": ".tactus/motivotest/20260905-investigation/small-input/.motivo-logs/stderr.log",
  "stdout_truncated": false,
  "stderr_truncated": false,
  "logs_may_be_partial": false
}
```

`status` is `exited`, `timed_out`, or `cancelled`. A command's ordinary nonzero
exit code is a **negative observation**, with `ok=true` for the plugin transport;
it must never be displayed as a passed test. An experiment terminated by the
deadline or caller returns null exit/signal information, partial-log status,
and null truncation flags because its worker could not finalize those facts.
Logs may not exist if preparation was interrupted before they were opened.

Invalid input, unsafe paths, missing tools, sandbox startup failure, or failed
supervision return terminal `ok=false` with an error code and message. Execution
errors include log paths in `error.details` when available. A successfully
started shell may itself return 126 or 127 (for example, from a missing nested
command); these remain negative observations and never imply a passed test.
Failure to build the sandbox never runs argv on the host.

## Verification

```sh
cargo test -p motivo-test
cargo clippy -p motivo-test --all-targets -- -D warnings
```

Linux integration tests use real Bubblewrap, not a fake backend. They verify
read-only project and host boundaries, allowed writes, environment removal,
network and Unix-socket isolation, unavailable namespace failure, symlink and
hard-link rejection, literal argv, protected bounded logs, shared deadlines,
setsid descendants, graceful cancellation, and abrupt parent death. These tests
require usable Linux user/PID/mount namespaces and installed system test tools
(`/usr/bin/python3`, `/bin/sh`, `setsid`, and `sleep`). They do not install any
dependency or contact a model.
