---
title: Agenstro 0.3 troubleshooting
status: alpha
last_verified: 2026-08-15
applies_to: "Clef 0.3.0.0 and Tactus 0.3.0"
platforms: [windows, ubuntu]
---

# Agenstro 0.3 troubleshooting

Start with the smallest failing boundary. Tactus can diagnose the workspace and
Haskell tools without contacting a provider:

```powershell
tactus --version
tactus doctor --root D:\path\to\project
tactus list --root D:\path\to\project
```

Add `--json` to `doctor`, `list`, `init`, `generate`, or `smoke` when structured
output is easier to inspect. `check` and `run` inherit the terminal so Cabal,
GHC, the Haskell program, and plugin diagnostics remain visible.

## Windows reports invalid UTF-8, mojibake, or a Unicode error

Symptoms include:

- Chinese or other non-ASCII prompt text becoming corrupted;
- `UnicodeDecodeError` or `UnicodeEncodeError` in a provider/effect host;
- `plugin stdout was not UTF-8 text`;
- a JSONL frame that is valid when written to a file but invalid through a
  Windows pipe; or
- a failure that appears only when a path, prompt, or provider response contains
  emoji or characters outside the active code page.

Check the current Python mode in the same terminal that launches Tactus:

```powershell
python -c "import locale,sys; print('utf8_mode=', sys.flags.utf8_mode); print('stdin=', sys.stdin.encoding); print('stdout=', sys.stdout.encoding); print('locale=', locale.getencoding())"
```

The current reference provider and effect hosts reconfigure their protocol
stdin/stdout to UTF-8 independently of this diagnostic. If the failure comes
from an older editable checkout or a custom Python plugin, set both variables
**before** starting it:

```powershell
$env:PYTHONUTF8 = "1"
$env:PYTHONIOENCODING = "utf-8"

python -c "import sys; print(sys.stdin.encoding, sys.stdout.encoding)"
tactus doctor --root D:\path\to\project
```

Environment assignments apply to the current PowerShell process and children;
repeat them in a new terminal. Changing only the console font does not change
redirected Python pipe encoding. A custom plugin in any language must read and
write UTF-8 regardless of the console code page.

If the current reference host still fails, confirm that `tactus`,
`tactus-provider-host`, and `tactus-effect-host` resolve to the same installation
before reporting a regression.

## `tactus` or the reference plugin hosts are not found

An editable Tactus install exposes three console commands:

```text
tactus
tactus-provider-host
tactus-effect-host
```

Activate the virtual environment or add its executable directory to `PATH`.
On Windows PowerShell, from the repository root:

```powershell
.\.venv\Scripts\python.exe -m pip install -e ".\tactus-runtime"
$env:PATH = (Resolve-Path .\.venv\Scripts).Path + ";" + $env:PATH

Get-Command tactus
Get-Command tactus-provider-host
Get-Command tactus-effect-host
```

Do not confuse `tactus-provider-host` with a native provider executable. The
host is the Agenstro adapter; `codex`, `claude`, or `opencode` must be installed
separately for that provider.

## `tactus init` cannot locate `clef-sdk`

Without `--sdk`, Tactus searches the project, its parent, the editable source
checkout, and `TACTUS_CLEF_SDK`. Checkout layouts can make that discovery
ambiguous. Pass the Cabal package directory explicitly:

```powershell
$sdk = (Resolve-Path D:\src\agenstro\clef-sdk).Path
tactus init D:\work\my-project --sdk $sdk
```

The path must name the directory containing `clef-sdk.cabal`, not its
`haskell/src` subdirectory. Tactus writes the resolved location into
`.tactus/cabal.project`.

For repeated use, the optional environment override is:

```powershell
$env:TACTUS_CLEF_SDK = (Resolve-Path D:\src\agenstro\clef-sdk).Path
```

## `tactus doctor` reports missing Cabal, GHC, or runghc

`doctor` requires all three tools on the effective `PATH`:

```powershell
Get-Command cabal
Get-Command ghc
Get-Command runghc
ghc --version
cabal --version
```

Install a compatible GHC/Cabal pair, restart or refresh the terminal after a
toolchain install, and run `doctor` again. Clef currently requires the GHC
package `base >=4.20 && <4.23`. A GHC outside that range produces a Cabal solver
error even if the executable itself is discoverable.

On Windows, Tactus also reads current user and machine `PATH` registry values to
help long-lived shells find a newly installed toolchain. This does not repair a
missing installation or an incompatible version.

## Cabal cannot build `clef-sdk`

First reproduce the package build from the Agenstro repository root:

```powershell
cabal build all
cabal test all --test-show-details=direct
```

Then inspect the project-local Cabal pointer:

```powershell
Get-Content D:\path\to\project\.tactus\cabal.project
```

Common causes are:

- `.tactus/cabal.project` points to a moved or deleted checkout;
- GHC's bundled `base` is outside the supported range;
- Cabal cannot obtain dependencies on the first build;
- a stale 0.2 `.tactus` directory was preserved by `init`; or
- the Haskell source imports a package that the project does not declare.

`tactus init` intentionally preserves existing files. Re-running it will not
silently replace a bad `cabal.project`; review and repair the file explicitly or
initialize a clean project.

## `check` says that no Haskell scripts were selected

`tactus check` considers `.hs` and `.lhs` files under `.tactus/scripts`.
`tactus run` selects only runnable entry names by default:

```text
010_plan.hs
020_review.lhs
```

The required entry pattern is a three-digit prefix, underscore, lowercase
alphanumeric slug (additional underscore-separated words are allowed), and
`.hs` or `.lhs`. Use `tactus list` to see each file's classification and
warning.

A helper can be checked or run by an explicit path even when it is not a
default entry:

```powershell
tactus check .tactus\scripts\Support.hs
tactus run .tactus\scripts\manual.hs -- --workflow-argument
```

Arguments after `--` are passed to the selected Haskell program.

## Offline smoke fails because a provider executable is missing

With no names, `tactus smoke` probes every configured provider and effect. The
default configuration therefore expects `codex`, `claude`, and `opencode` all
to exist. Select only the component being diagnosed:

```powershell
tactus smoke codex
tactus smoke claude-code
tactus smoke opencode
tactus smoke workspace.paths
```

The default provider smoke is offline in the sense that it runs the native
CLI's version command and sends no model prompt. It still starts a local
executable. `--live` is the separate, explicit model-call path.

If the native command works in one terminal but not through Tactus, compare
`PATH` and provider-specific environment variables in the process that launches
Tactus. Commands inherit the current environment; Tactus does not copy login
state between users, shells, containers, or machines.

## A provider is installed but live smoke or generation fails

Verify the native CLI outside Agenstro first. Authentication commands and
account policy are owned by that CLI and may change independently of Tactus.
Then run a selected offline probe before any live probe:

```powershell
tactus smoke codex
tactus smoke codex --live --json
```

For generation, retain the structured result:

```powershell
tactus generate --provider codex --json "Create one minimal Haskell workflow."
```

Check these boundaries:

- the provider name is present in `.tactus/tactus.toml`;
- its `command` is a non-empty argument array;
- the native executable is available on inherited `PATH`;
- the native CLI is authenticated for the intended account and endpoint;
- configured `model`, `effort`, or OpenCode `variant` values are supported by
  that native version; and
- `extra_args`, `extra_env`, or a `command_prefix` have not changed native
  behavior unexpectedly.

CI covers adapter argument construction and output parsing with fakes. It does
not prove that a current native CLI, account, model, or managed policy will
accept a live request.

## OpenCode still asks, denies, or changes behavior under `--auto`

The OpenCode adapter uses `opencode run --auto --format json` and injects an
inline `permission=allow` value. This is not equivalent to the explicit bypass
flags used by the Codex and Claude Code adapters. An explicit deny or managed
configuration can still win.

Inspect the effective OpenCode configuration and organization policy. Do not
weaken a deny merely to make Agenstro report parity with another provider. The
documented support boundary is that full OpenCode approval bypass cannot be
proven.

## Generation returns success but no runnable script appears

`generate` asks the provider to edit `.tactus/scripts` and then performs
discovery; it does not synthesize a file from provider text. A provider can
return successfully without following the file-writing instruction.

1. Re-run `tactus list` and inspect helpers as well as entries.
2. Inspect `.tactus/PROMPT.md` and the requested goal.
3. Use `generate --json` to retain provider events, effect evidence, and the
   discovered script list.
4. Inspect the workspace diff before retrying.
5. Rename a valid program to `NNN_slug.hs` only after reviewing its contents.

Do not automatically run an unreviewed file merely because the provider process
exited successfully.

## A custom plugin emits invalid protocol output

Protocol stdout may contain only UTF-8 JSONL event frames and one terminal
result. Send logs to stderr. Common violations are:

- banners, progress bars, or debug prints on stdout;
- an incorrect correlation `id`;
- duplicate JSON object keys;
- two result frames or data after the result;
- no terminal result before exit;
- NaN or infinite JSON numbers;
- a success result followed by a non-zero process exit; or
- output encoded with the Windows locale instead of UTF-8.

Reduce the plugin to one `describe` or offline `smoke` request and compare it to
the [local plugin protocol](reference/plugin-protocol-v1.md). A valid structured
`ok:false` result is a plugin-reported failure, not a reason to print an
unstructured replacement message.

## A provider invocation hangs

Normal provider invocation has no default framework deadline. A trusted local
plugin or descendant that never closes its output can block the caller. An
adapter-specific timeout can be configured in `.tactus/tactus.toml`:

```toml
[providers.codex.options]
timeout_seconds = 600
```

This bounds the adapter's direct native CLI wait. It is not a hard process-tree
deadline: a descendant retaining a pipe can delay completion, and termination
after an external request may leave its outcome unknown. Choose retry behavior
only after deciding whether repeating the provider operation is safe.

## `workspace.paths` missed or misattributed a change

The effect compares snapshots. It can report final added, modified, deleted,
and type-changed paths, but it cannot observe:

- reads;
- a file created and deleted between snapshots;
- the identity of the process that made a change;
- content under `.git`, `.tactus/path-effect`, or
  `.tactus/dist-newstyle`; or
- authorization, intent, or whether a change should be accepted.

Concurrent editors and background tools can appear in the same diff. Treat the
result as workspace evidence, not agent attribution, a sandbox decision, or a
rollback log.

## `run` made an unexpected model call or filesystem change

`tactus check` type-checks; it does not execute. `tactus run` executes ordinary
trusted Haskell. Inspect the selected scripts for `invoke`, `invokeWith`,
`perform`, `liftIO`, process launches, and direct filesystem operations. Also
inspect the selected provider/effect commands in `.tactus/tactus.toml`.

There is no general dry-run mode for arbitrary Haskell `IO`. Use a disposable
workspace and non-live/local test plugins when execution effects are uncertain.

## Old Motivo, Segno, worker, or daemon instructions do not work

Motivo Studio and Segno Flow are frozen 0.2 surfaces. The old Clef Python
builder, Tactus cells/Jupyter worker, checkpoint/CAS path, and daemon commands
are not part of the 0.3 install. Use the current commands:

```text
tactus init
tactus list
tactus prompt
tactus generate
tactus check
tactus run
tactus doctor
tactus smoke
```

For a side-by-side transition, follow the
[0.2 to Haskell 0.3 migration guide](migrations/0.2-to-haskell-0.3.md). Do not
point 0.3 at old daemon state or assume an in-place state migration.

## Before reporting a defect

Capture only non-secret diagnostics:

- operating system and architecture;
- `tactus --version`, `python --version`, `ghc --version`, and
  `cabal --version`;
- `tactus doctor --json`;
- the relevant `tactus list --json` or selected `smoke --json` result;
- whether the operation was offline, live smoke, generation, or Haskell run;
- the minimal script or redacted plugin frames needed to reproduce it; and
- whether the failing host was the current reference implementation, an older
  editable checkout, or a custom plugin.

Do not attach access tokens, provider configuration containing secrets, raw
environment dumps, or private prompts unless their disclosure has been reviewed.
