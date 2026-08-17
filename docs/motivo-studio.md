# Motivo Studio

Motivo Studio `0.3.0` is a TypeScript, React, and Electron visualization layer
for an installed Rust Tactus runtime. It shows workspace health, ordered Haskell
workflow entries, configured plugins, command output, and factual invocation
traces. It does not implement workflow semantics itself.

## Install and start on Windows x64

Install Tactus first. Node.js 22.12 or newer is required to build Motivo from
this checkout. From the repository root, install the locked dependencies and
the current Windows x64 desktop build:

```powershell
cargo install --path tactus-runtime --bin tactus --locked --force
npm --prefix motivo-studio ci
npm --prefix motivo-studio run install:windows
```

The installer replaces `%LOCALAPPDATA%\Programs\MotivoStudio` and adds that
directory, which contains `motivo-studio.exe`, to the user `PATH`. It does not
publish or globally install an npm package. Open a new terminal to receive the
`PATH` change, then start the UI or open one initialized workspace:

```powershell
motivo-studio
motivo-studio 'D:\work\Project with spaces'
```

The command shape is `motivo-studio [WORKSPACE]`. It resolves a relative path
from the calling terminal. The equivalent explicit form is `--workspace PATH`;
use `--` before a path whose first character is `-`. Supply exactly one
workspace and do not combine the positional and explicit forms.

Motivo is single-instance. A second invocation without a workspace focuses the
existing window. With a workspace, Tactus validates and switches the existing
window before Motivo focuses it; invalid roots leave the current workspace open,
show an error, and focus that same window.

Close Studio and run `npm --prefix motivo-studio run install:windows` again
after updating the checkout to upgrade. Close Studio and uninstall the
recognized per-user application and its user-`PATH` entry with:

```powershell
npm --prefix motivo-studio run uninstall:windows
```

Development remains separate from the installed launcher:

```powershell
npm --prefix motivo-studio start
```

By default Electron resolves `tactus` from `PATH`. A development or installed
application may point at an exact binary by setting `MOTIVO_TACTUS_BIN` before
starting Studio.

Use **Open workspace** for a project that already contains `.tactus`, or
**Initialize folder** to run `tactus init` and then open it. Initialization
still needs a discoverable Clef SDK; when automatic checkout discovery is not
available, initialize from the CLI with `tactus init --sdk <clef-sdk>` first.

## Views and actions

The application has four projections:

- **Overview** summarizes Tactus doctor checks, workflow entries, registries,
  recent runs, and the active action.
- **Workflow** displays numbered runnable entries in Tactus order and separates
  helper modules. It can request Generate, Check, or Run.
- **Plugins** groups providers, effects, and open generic plugins. It shows only
  redacted metadata and can start offline Smoke. Live Smoke is a separate,
  explicit action because it may contact or bill a provider.
- **Runs** pages through open `agenstro.trace/v1` events and shows the terminal
  summary when one exists. Canonical Tactus presentation messages form the
  visible log; unknown and legacy event data remains available in collapsed
  technical details.

The action drawer displays only `[state]`, `[info]`, `[warning]`, and `[error]`
plus bounded natural-language messages while Electron owns the top-level
Tactus child. It strictly recognizes those exact tags from Tactus human output;
untagged stdout/stderr stays inside a collapsed raw-output panel because stderr
alone does not imply an error. If that raw projection reaches its byte or frame
budget, Electron continues draining both pipes, omits subsequent raw frames,
and emits one `[warning]`; it does not kill Tactus or replace the child's real
terminal status. Only one action may run at a time. Closing the window or
pressing Cancel asks that process to stop; a terminal action event reports the
observed result. Cancellation is best effort at the desktop boundary, while
Tactus remains responsible for supervised plugin descendants.

The window opens at 120% zoom. `Ctrl` + mouse wheel and `Ctrl` + `+` / `-`
adjust the complete interface between 80% and 200%; `Ctrl` + `0` restores the
120% default.

## Typed boundary

The renderer is sandboxed, has no Node integration, and cannot access host paths
or arbitrary Electron IPC. A context-isolated preload exposes one named,
Zod-validated method for each supported operation. Electron main retains the
workspace root and starts Tactus with an argv array and `shell: false`.

Motivo never reads `tactus.toml`, `runtime.json`, or `.tactus/runs` directly.
The Rust runtime owns sorting, registry resolution, trace validation, and
resource limits through:

```powershell
tactus studio inspect --root D:\work\project --run-limit 50
tactus studio events run-... --root D:\work\project --after 0 --limit 250 --max-bytes 4194304
```

Each query emits exactly one `tactus.control/v1` envelope containing an
`agenstro.studio/v1` projection. The projection omits configured command arrays,
plugin options, generation prompt text, and absolute script paths. Timestamps,
event sequences, and counts are decimal strings, avoiding JavaScript's 53-bit
integer limit. Event pages report `ok`, `partial`, or `corrupt` integrity and
refuse unsafe trace paths. An event may carry Tactus's canonical
`presentation` object with one public category and a bounded natural-language
message; its structured `data` remains redacted technical evidence. Trace pages
are diagnostic observations, not replay input or authoritative workflow state.
See the
[Studio control API v1 reference](reference/studio-control-v1.md) for the exact
envelopes, pagination rules, and limits.

## Intentional non-goals

Studio does not provide:

- a daemon, scheduler, replay engine, artifact store, checkpoint, or rollback;
- a general terminal or shell;
- source editing or a second script-discovery implementation;
- provider login, credential storage, permission policy, or authentication;
- a guarantee that cancelling a top-level command reverses external work.

Workflows and plugins remain trusted local code with the capabilities described
in the [support matrix](reference/support-matrix.md).

## Verify locally

The default tests use fake Tactus processes and never contact a model:

```powershell
npm --prefix motivo-studio run format:check
npm --prefix motivo-studio run lint
npm --prefix motivo-studio run typecheck
npm --prefix motivo-studio test
npm --prefix motivo-studio run package
```

Packaging creates an unsigned current-platform application. The package does
not bundle a Tactus executable; the target machine must install Tactus or set
`MOTIVO_TACTUS_BIN`.
