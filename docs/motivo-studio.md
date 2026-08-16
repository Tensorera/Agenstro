# Motivo Studio

Motivo Studio `0.3.0` is a TypeScript, React, and Electron visualization layer
for an installed Rust Tactus runtime. It shows workspace health, ordered Haskell
workflow entries, configured plugins, command output, and factual invocation
traces. It does not implement workflow semantics itself.

## Install and start

Install Tactus first, then install the locked Node dependencies:

```powershell
cargo install --path tactus-runtime --bin tactus --locked --force
npm --prefix motivo-studio ci
npm --prefix motivo-studio start
```

Node.js 22.12 or newer is required. By default Electron resolves `tactus` from
`PATH`. A development or packaged installation may point at an exact binary by
setting `MOTIVO_TACTUS_BIN` before starting Studio.

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
  summary when one exists. Unknown event kinds remain visible as open JSON.

The action drawer displays bounded stdout and stderr while Electron owns the
top-level Tactus child. Only one action may run at a time. Closing the window or
pressing Cancel asks that process to stop; a terminal action event reports the
observed result. Cancellation is best effort at the desktop boundary, while
Tactus remains responsible for supervised plugin descendants.

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
refuse unsafe trace paths. See the
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
