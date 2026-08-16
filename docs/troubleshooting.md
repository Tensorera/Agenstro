---
title: Agenstro 0.3 troubleshooting
status: alpha
last_verified: 2026-08-15
applies_to: "Clef Haskell 0.3.0.0 and Tactus Rust 0.3.0"
---

# Agenstro 0.3 troubleshooting

Start with the installed binary, workspace discovery, and typed diagnostics:

```powershell
Get-Command tactus
tactus --version
tactus doctor --root D:\path\to\project --json
tactus list --root D:\path\to\project --json
```

The expected Tactus version is `0.3.0`. The current runtime is one Rust binary;
there are no separate provider/effect host executables to locate.

## `tactus` is missing or an older command is selected

Install the current source explicitly, then open a terminal whose Cargo bin
directory is on `PATH`:

```powershell
Set-Location D:\src\Agenstro
cargo install --path tactus-runtime --bin tactus --locked --force
Get-Command tactus
tactus --version
```

If multiple results appear, remove the stale path from the current terminal or
invoke the Cargo-installed executable by its full path. An old command surface
that mentions workers, cells, notebooks, or a daemon is not Tactus Rust `0.3`.

## `tactus init` cannot locate `clef-sdk`

Pass the checkout explicitly. The directory must contain `clef-sdk.cabal`:

```powershell
$sdk = (Resolve-Path D:\src\Agenstro\clef-sdk).Path
tactus init D:\work\my-project --sdk $sdk
```

Initialization never overwrites an existing `.tactus/cabal.project`. If that
file points at a moved checkout, edit its package path deliberately or
initialize a new disposable project.

## `doctor` reports missing Cabal, GHC, or runghc

Verify all three names in the same terminal that launches Tactus:

```powershell
Get-Command cabal
Get-Command ghc
Get-Command runghc
ghc --print-libdir
cabal --version
```

On Windows, a GHCup installation commonly needs `C:\ghcup\bin` and
`C:\cabal\bin` on `PATH`. Restart long-lived terminals after changing PATH.
Clef requires a GHC whose bundled `base` satisfies the bounds in
`clef-sdk/clef-sdk.cabal`.

## Configuration is rejected before a command runs

Tactus parses `.tactus/tactus.toml` into distinct provider, effect, and generic
plugin definitions. Frequent errors are:

- `api` is not `clef.runtime/v1`;
- `default_provider` has no matching `[providers.<name>]` table;
- `command` is empty or written as a shell string instead of an argv array;
- a provider-only field such as `model` was placed on an effect/plugin;
- an unknown field was added beside, rather than inside, open `options`; or
- options contain a TOML datetime, NaN, or infinity, which cannot cross the JSON
  runtime boundary.

A minimal generic plugin is:

```toml
[plugins.example]
command = ["example-plugin", "--jsonl"]

[plugins.example.options]
feature = "open-value"
```

Use `tactus doctor --json` after every configuration edit.

## A plugin name is unknown or ambiguous

The same text may exist in `[providers]`, `[effects]`, and `[plugins]`. Specify
the registry:

```powershell
tactus smoke provider:codex
tactus smoke effect:workspace.paths
tactus plugin-call example describe --namespace plugin
```

Without a prefix/namespace, auto-resolution succeeds only when exactly one
registry contains the name.

## `list`, `check`, and `run` disagree about scripts

`check` considers every `.hs`/`.lhs` source below `.tactus/scripts` by default.
`run` implicitly selects only names matching `NNN_slug.hs` or
`NNN_slug.lhs`, ordered by prefix and path. Other modules are helpers.

```powershell
tactus list
tactus check .tactus\scripts\Support.hs
tactus run --script .tactus\scripts\010_main.hs -- --argument
```

`run` uses a repeatable `--script`; it does not accept an explicit entry as a
bare positional path. Re-running `init` will preserve, not replace, an old
prompt or script tree.

## Cabal or GHC fails during `check`

Run the local project command directly to separate dependency resolution from
Tactus orchestration:

```powershell
cabal build --project-dir .tactus lib:clef-sdk
```

Then inspect `.tactus/cabal.project`, the SDK path, network/proxy access to
Hackage, and the selected GHC. `tactus check --keep-going` continues to later
sources after a source-specific failure; it cannot make a failed Clef package
build usable.

## Rust validation fills the repository

Do not build into the default workspace `target/`. Use a unique system
temporary target and always clean it:

```powershell
$targetDir = Join-Path $env:TEMP ("agenstro-target-" + [guid]::NewGuid().ToString("N"))
$env:CARGO_TARGET_DIR = $targetDir
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo check -p tactus-runtime --all-targets --locked
  cargo test -p tactus-runtime --locked
} finally {
  cargo clean --target-dir $targetDir
  Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}
```

Use `cargo clean --target-dir <exact-temp-path>` for cleanup; verify the target
path before cleaning it. Package-scoped commands are usually sufficient while
iterating on Tactus.

## Chinese text or emoji produces a protocol error

`agenstro.plugin/v1` is always UTF-8. Rust Tactus reads stdout as bytes and
decodes each complete line strictly, so it does not depend on the Windows
console code page.

If a custom plugin fails, make the plugin explicitly read/write UTF-8 protocol
data and flush after each JSONL record. Human-readable logs belong on stderr.
Do not encode protocol stdout using an OEM/ANSI locale. The usual error includes
the byte position at which UTF-8 became invalid.

## Events appear only at the end or never appear

Tactus and Clef route complete LF-terminated frames incrementally. Check that
the plugin:

- writes each event as one complete JSON object followed by `\n`;
- flushes stdout after the line;
- does not wait to fill a large userspace buffer;
- uses the active request `id`; and
- sends diagnostics, banners, and progress bars to stderr.

The event queue is bounded. If a sink stops consuming, Tactus fails the
invocation after the queue or delivery deadline is exhausted and cleans the
owned process group; it does not let a blocked sink disable cancellation. A
partial line is not an event and will not be exposed before its newline.

## A custom plugin reports a protocol failure

Protocol stdout permits zero or more event frames followed by exactly one
terminal result. Common violations are:

- non-UTF-8 data or non-JSON banners on stdout;
- duplicate JSON object keys or non-finite numbers;
- an incorrect correlation ID;
- a second result or any frame after the terminal result;
- success without `value`, failure without a structured `error`, or no terminal
  result before exit;
- a frame/request larger than the supervisor limit; or
- a successful terminal result followed by an incoherent non-zero exit.

Reduce the call to `describe` and retain its structured report:

```powershell
tactus plugin-call example describe --namespace plugin --json
```

Compare stdout with the [local plugin protocol](reference/plugin-protocol-v1.md).

## A provider is installed but smoke/generation fails

First run an offline selected probe, then inspect the native CLI outside
Agenstro:

```powershell
tactus smoke provider:codex --json
codex --version
```

Only after the offline boundary works, opt into a real request:

```powershell
tactus smoke provider:codex --live --json
tactus generate --provider codex --json "Create one minimal typed workflow."
```

Verify:

- the provider registry key and command;
- native executable visibility on inherited `PATH`;
- native login/account/endpoint state;
- current model and effort/variant support; and
- provider-specific `extra_args`, `extra_env`, or command-prefix options.

Fake-driven tests verify adapter translation, not live account acceptance or a
future native CLI version.

## OpenCode still asks or denies under `--auto`

The adapter uses `opencode run --auto --format json` and injects an inline
`permission=allow` configuration. This is not equal to the explicit dangerous
bypass flags used by Codex and Claude Code. An explicit deny or managed policy
can still win.

Inspect the effective OpenCode configuration and organization policy. The
supported claim is `full_bypass=false`, not permission parity.

## Generation succeeds but no runnable entry appears

`generate` asks the provider to write `.tactus/scripts`; it does not convert the
provider's final text into a file. A provider can return successfully without
following that instruction.

1. Run `tactus list` and inspect helpers as well as entries.
2. Read `.tactus/PROMPT.md` and the requested goal.
3. Inspect the `--json` generation report and relevant run journal.
4. Review the `workspace.paths` evidence.
5. Only after review, give a valid program an `NNN_slug.hs` name.

Never automatically run an unreviewed file just because the provider process
exited successfully.

## A plugin/provider invocation times out or is cancelled

Tactus puts the command in a Unix process group or Windows Job Object and
terminates the owned group on deadline, Ctrl+C cancellation, or protocol
failure. Windows Job Objects contain the nested tree. On Unix, a process that
deliberately creates a new session can escape process-group containment; do not
treat this mechanism as a hostile-code sandbox.

Most public invocation commands use `--timeout-seconds`; the default is 1,800
seconds and `0` disables the deadline.

Process termination is not remote rollback. A model service may have received
or completed work before the local timeout. Inspect the journal and workspace
before deciding whether a retry is safe.

## A run journal is missing, incomplete, or contains sensitive text

Each supervised plugin call uses `.tactus/runs/<run-id>/events.jsonl` and
`summary.json`. Event lines are flushed immediately. The summary appears only
after the terminal outcome and is published by atomic rename.

If the machine loses power, a run can legitimately have events without a
summary. Journals can include prompts, raw provider events, errors, and path
evidence; there is no built-in redaction or credential store. Do not commit or
attach them without reviewing their contents.

`agenstro.trace/v1` is diagnostic evidence, not deterministic replay, a CAS, or
a rollback log.

## `workspace.paths` missed or misattributed a change

The effect compares snapshots and reports final added, modified, deleted, and
type-changed paths. It cannot observe:

- reads or transient create/delete cycles;
- which process made a concurrent change;
- content below `.git`, effect-internal state/run data, `target`,
  `node_modules`, `build`, or `dist-newstyle`; or
- authorization, intent, or whether a change should be accepted.

A snapshot fails after 100,000 paths, 512 MiB hashed, or 30 seconds. The effect
stores hashes/metadata for comparison, not restorable file content. Treat
the result as a workspace delta, not a security audit or transaction.

An interrupted `observe.end` can be retried with the same opaque begin token;
Tactus returns the atomically committed delta when one exists. Completion
metadata is retained for up to 24 hours and cleaned under bounded work, so it
should not be treated as permanent trace storage.

## `run` made an unexpected call or filesystem change

`check` compiles; `run` executes ordinary trusted Haskell. Inspect selected
scripts for `invoke`, `call`, `perform`, `liftIO`, process launches, and direct
filesystem operations. Inspect `.tactus/tactus.toml` for the exact commands.

There is no general dry-run for arbitrary Haskell `IO`, no authorization layer,
and no rollback. Use reviewed scripts, fake plugins, and a disposable workspace
when effects are uncertain.

## Motivo cannot open a workspace

Motivo `0.3` invokes an external Rust `tactus` executable. Confirm that the
same environment can run:

```powershell
tactus --version
tactus studio inspect --root D:\path\to\project
```

If Studio is launched outside that environment, set `MOTIVO_TACTUS_BIN` to the
exact executable path before starting it. **Open workspace** requires an
initialized `.tactus`; use **Initialize folder**. When SDK discovery is not
available, initialize from a terminal first:

```powershell
tactus init D:\path\to\project --sdk D:\path\to\clef-sdk
```

A moved checkout may require repairing the Clef SDK link.

`partial` run integrity means the trace is still open or the bounded page ended
before the current file. `corrupt` means Rust rejected a complete trace record;
Motivo does not guess around it. Run `tactus doctor` and preserve a redacted run
ID when reporting the defect.

## Old Segno, worker, or daemon instructions fail

Segno Flow, notebook/cell workers, daemon commands, persistent scheduling,
artifact/CAS, and checkpoint restore are not part of the current runtime. The
supported CLI is:

```text
tactus init
tactus list
tactus prompt
tactus generate
tactus check
tactus run
tactus doctor
tactus smoke
tactus plugin-call
tactus studio inspect
tactus studio events
```

Use the [0.2 to Haskell 0.3 migration
guide](migrations/0.2-to-haskell-0.3.md) only as side-by-side migration context;
do not point the Rust runtime at old daemon state.

## Before reporting a defect

Capture only non-secret diagnostics:

- operating system and architecture;
- `tactus --version`, `rustc --version`, `ghc --version`, and
  `cabal --version`;
- `tactus doctor --json`;
- relevant `list --json`, `smoke --json`, or `plugin-call --json` output;
- the exact command and whether it was offline or live;
- a minimal script or redacted protocol frames; and
- the corresponding run ID and redacted summary.

Do not attach tokens, raw environment dumps, private prompts, or unreviewed
journals.
