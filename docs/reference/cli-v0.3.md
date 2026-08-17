---
title: Agenstro CLI reference
status: alpha
last_verified: 2026-08-16
applies_to: "Tactus/Motivo Studio 0.3.0 and Segno 0.3.0"
---

# Agenstro CLI reference

This is the supported human-facing command surface for the 0.3 source alpha.
It was checked against the compiled `--help` output on 2026-08-16. Internal
`dispatch`, `provider-host`, and `effect-host` commands implement plugin
plumbing and are not a user API.

## Common path behavior

Most commands accept `--root PATH`. Tactus and Segno search upward from that
path for `.tactus/tactus.toml`, so a workspace descendant is sufficient.
Commands that emit `--json` reserve stdout for machine data; diagnostics go to
stderr. On PowerShell, pass JSON params in single quotes. Quote any path that
contains spaces.

## Tactus

| Command | Purpose | Important options |
| --- | --- | --- |
| `tactus init [ROOT]` | Idempotently create `.tactus` | `--sdk PATH`, `--json` |
| `tactus list` | List ordered entries and helpers | `--root`, `--json` |
| `tactus prompt` | Print resolved generation instructions | `--root` |
| `tactus doctor` | Validate config, GHC/Cabal, SDK, and plugin commands | `--root`, `--json` |
| `tactus runtime-json` | Print Clef's normalized runtime config | `--root` |
| `tactus check [SCRIPT...]` | Compile-check without executing | `--package NAME`, `--keep-going`, `--timeout-seconds` |
| `tactus run` | Execute selected or ordered entry scripts | repeatable `--script`, `--package NAME`, `--keep-going`, `-- ARG...` |
| `tactus generate GOAL...` | Ask one provider to write numbered Haskell scripts | `--provider NAME`, `--timeout-seconds`, `--json` |
| `tactus plugin-call NAME METHOD` | Invoke a registry entry directly | `--namespace`, `--params JSON`, `--timeout-seconds`, `--json` |
| `tactus smoke [NAME...]` | Probe configured plugins | `--live`, `--json` |
| `tactus studio inspect` | Return a redacted Studio workspace projection | `--run-limit` |
| `tactus studio events RUN_ID` | Read one bounded trace page | `--after`, `--limit`, `--max-bytes` |

Without `--json`, the Tactus-generated user-log vocabulary is closed to
`[state]`, `[info]`, `[warning]`, and `[error]`, each followed by bounded
natural-language text. Native stderr, provider JSON/free text, event payloads,
stable codes, and counters are technical diagnostics and are never promoted
directly into that layer. `check`/`run` may attach compiler or workflow process
output separately. Machine-mode JSON remains structured by design.

Clef is always exposed to `check` and `run`. Repeat `--package` for extensions,
for example Segno:

```powershell
tactus check --root D:\work\project --package segno-flow `
  .tactus\scripts\900_record_active_window.hs

tactus run --root D:\work\project --package segno-flow `
  --script .tactus\scripts\900_record_active_window.hs
```

`generate` and `smoke --live` may contact or bill a provider. Plain `smoke` is
an offline executable/capability probe. `check` does not execute workflow code;
`run` executes trusted Haskell and may perform arbitrary `IO`.

Registry names can collide. Use an explicit namespace when needed:

```powershell
tactus plugin-call workspace.paths describe --namespace effect --params '{}'
tactus plugin-call calculator add --namespace plugin `
  --params '{"left":19,"right":23}' --json
```

## Segno

| Command | Purpose | Important options |
| --- | --- | --- |
| `segno init` | Create `.tactus/segno`, register built-ins, link the package | `--root`, `--sdk` |
| `segno install SCRIPT` | Compile the task manifest and install it | `--root`, `--sdk`, `--task-timeout-seconds` |
| `segno list` | List installed persistent tasks | `--root`, `--json` |
| `segno once` | Poll triggers and drain runnable occurrences once | `--root`, `--json`, `--task-timeout-seconds` |
| `segno driver` | Run the single-node wake/poll/execute loop | `--root`, `--poll-seconds`, `--task-timeout-seconds` |
| `segno status` | Inspect runtime-owned lifecycle | `--job TASK`, `--json` |
| `segno history` | Inspect lifecycle or business-state history | `--state-key`, `--occurrence`, `--limit`, `--json` |

The task timeout is per Tactus build/run phase, defaults to 1,800 seconds, and
accepts 1–604,800 seconds. Segno derives a longer Running lease from it.

`history --state-key` and `history --occurrence` are mutually exclusive.
Business-state history can contain plugin values. Tactus journals persist only
a bounded diagnostic projection: prompt/provider raw text, terminal success
values, and native stderr are redacted or summarized, but errors, hashes, and
path metadata can still be sensitive.

## Motivo Studio on Windows x64

The installed desktop launcher has one optional workspace argument:

```text
motivo-studio [WORKSPACE]
```

With no argument it opens or focuses the Studio window. A positional path opens
that initialized Tactus workspace; relative paths are resolved from the calling
terminal's current directory. Quote paths that contain spaces:

```powershell
motivo-studio 'D:\work\Project with spaces'
```

`motivo-studio --workspace PATH` is the equivalent explicit form. Use `--`
before a path whose first character is `-`. Pass exactly one workspace and do
not combine the positional and `--workspace` forms.

Motivo is single-instance. Running the command again without a workspace focuses
the existing window. With a workspace, Tactus validates and switches that window
before Motivo focuses it; a failed validation keeps the current workspace. See
[Motivo Studio](../motivo-studio.md) for the Windows installation and
development commands.

## Exit and outcome semantics

A zero process exit means the CLI completed its own operation. Domain data must
still be inspected: `doctor` can report unhealthy checks, and a Segno summary
can contain failed or `outcome_unknown` occurrences.

`OutcomeUnknown` means an external effect may have happened but no trustworthy
terminal result was obtained. Segno does not retry it automatically. Version
0.3 has no mutation command to resolve it; inspect lifecycle history and local
run evidence before deciding how to reconcile the external system.

## Version checks

```powershell
tactus --version
segno --version
ghc --numeric-version
cabal --numeric-version
```

Use [Installation](../install.md) for deployment, [First workflow](../getting-started.md)
for the initial tutorial, and [Troubleshooting](../troubleshooting.md) for
failure diagnosis.
