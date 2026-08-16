# Motivo Studio 0.3

Motivo Studio is the small desktop control plane for a Tactus workspace. It is a
TypeScript, React, Vite, and Electron application; Tactus remains the only owner
of workflow discovery, plugin configuration, health checks, execution, and run
history.

The first 0.3 release intentionally has no editor, terminal, daemon, scheduler,
recovery manager, gRPC client, protobuf-generated client, or direct journal
reader. It visualizes the versioned `tactus.control/v1` API and launches normal
Tactus CLI actions.

## Try it

Requirements:

- Node.js 22.12 or newer
- the Rust `tactus` executable on `PATH`
- an installed Haskell toolchain for `Check` and `Run`

From this directory:

```powershell
npm install
npm start
```

For a development build of Tactus that is not on `PATH`, set an absolute
executable path in the main-process environment before starting Studio:

```powershell
$env:MOTIVO_TACTUS_BIN = 'D:\path\to\tactus.exe'
npm start
```

Use **Open workspace** for a folder that already contains `.tactus`. Use
**Initialize folder** to run `tactus init` in the selected folder and then open
its control snapshot. Initialization must be able to discover Clef through the
normal Tactus rules, such as `TACTUS_CLEF_SDK`.

## Views

- **Overview** shows workspace health, script and plugin totals, and recent run
  outcomes.
- **Workflow** shows Tactus-ordered Haskell scripts and starts Generate, Check,
  or Run. Generate accepts an optional registered provider.
- **Plugins** shows provider, effect, and generic-plugin projections and starts
  offline or live smoke probes.
- **Runs** pages through the typed event projection for one opaque run ID. Open
  future event kinds remain visible as structured JSON.

One Generate, Check, Run, or Smoke action may be active at a time. Its bounded
stdout/stderr frames are projected to the renderer in real time. Cancel asks the
main process to terminate that Tactus process. When the action finishes, the
renderer refreshes the Studio snapshot.

Cancellation uses Electron/Node's platform process termination and is
best-effort. Tactus supervises the subprocesses it starts, but forcibly killing
the Tactus leader is not a universal process-tree guarantee on every operating
system. Do not treat Studio cancellation as a security boundary for untrusted
plugins.

## Authority boundary

```text
React renderer
    | strict Zod IPC; no path, Node, Electron, network, or filesystem authority
    v
Electron preload (context isolation + sandbox)
    |
    v
Electron main -- owns the selected absolute root and child process
    | argv array, shell: false, bounded stdout/stderr
    v
tactus studio inspect / studio events / init / generate / check / run / smoke
```

The renderer receives a random workspace handle and a redacted
`agenstro.studio/v1` snapshot. It never supplies or receives the absolute root.
The main process never reads `tactus.toml`, `runtime.json`, scripts, or journal
files. Control JSON is size-limited and validated before it crosses IPC. Action
output and open event payload strings are scrubbed for the selected root as a
second line of defense.

Electron uses context isolation, renderer sandboxing, no Node integration, a
locked-down `motivo://app` asset protocol, denied navigation/window creation,
and denied permissions. Tactus is external by design and is not bundled into the
desktop package.

## Development checks

```powershell
npm run typecheck
npm run lint
npm test
npm run format:check
```

The unit suite uses fake IPC and pure argv-contract tests; it does not call a
real model provider. Electron packaging is available through `npm run package`
or `npm run make`.
