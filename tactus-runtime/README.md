# Tactus Runtime

Tactus is the project-local command-line host for Clef Haskell workflow
scripts. It deliberately keeps the runtime surface small: initialize a
workspace, discover scripts, compile-check them, run them, and probe configured
provider/effect plugins.

## Workspace

From a project directory whose sibling checkout contains `clef-sdk`:

```powershell
tactus init
```

For another checkout layout, provide the SDK explicitly:

```powershell
tactus init --sdk D:\src\Agenstro\clef-sdk
```

Initialization creates only missing files and never replaces existing content:

```text
.tactus/
  tactus.toml       # provider/effect argv and defaults
  cabal.project     # points Cabal at clef-sdk
  PROMPT.md         # instructions injected into the runtime config
  scripts/          # ordinary .hs and .lhs files
```

The plugin protocol is always UTF-8. The bundled provider and effect hosts
configure their real standard input/output streams as UTF-8, including when
Windows Python starts under a legacy console code page. For an older editable
install or a third-party Python plugin that has not done so, this compatibility
fallback can be set before Tactus starts:

```powershell
$env:PYTHONUTF8 = "1"
$env:PYTHONIOENCODING = "utf-8"
```

Runnable entries use `NNN_slug.hs` or `NNN_slug.lhs`. Tactus orders them by
the three-digit prefix and then relative path. Other Haskell files are helpers:
they are reported with a warning and included by `check`, but are not run by
default. An explicit path may name any Haskell source file.

```powershell
tactus list
tactus prompt
tactus generate "inspect this project and implement the requested workflow"
tactus check
tactus run
tactus run .tactus\scripts\010_plan.hs -- --workflow-argument
tactus doctor
tactus smoke
tactus smoke codex --live
```

`check` builds `clef-sdk` and invokes GHC with `-fno-code`. `run` executes each
selected program with `runghc`. Both are fail-fast unless `--keep-going` is
provided, inherit the terminal and process environment, and set
`TACTUS_RUNTIME_CONFIG` to the absolute path of `.tactus/runtime.json`.

`generate` injects `PROMPT.md` plus the requested goal into the selected
provider's `invoke` method. The agent writes one or more numbered programs
directly into `.tactus/scripts`; Tactus then lists the files and does not run
them. Use `--provider NAME` to override `default_provider`.

## Plugin boundary

Provider and effect commands are argv arrays in `.tactus/tactus.toml`; no shell
parsing is involved. `smoke` starts each plugin as a one-shot JSONL process and
sends:

```json
{"api":"agenstro.plugin/v1","id":"...","method":"smoke","params":{"live":false}}
```

The process must emit exactly one matching terminal message:

```json
{"type":"result","id":"...","ok":true,"value":{}}
```

The default configuration contains `codex`, `claude-code`, and `opencode`
providers plus the `workspace.paths` effect. Higher-level provider behavior and
effect policy remain plugin concerns rather than Haskell DSL constraints.

The earlier worker, script-cell, Jupyter, and embedded Studio implementation
has moved to `../archive/tactus-runtime-0.2/`. It is outside both normal and
editable `tactus-runtime` installs; the 0.3 package exposes only the workspace
CLI and the two reference plugin hosts.
