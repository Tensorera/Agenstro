---
title: Agenstro CLI reference
status: alpha
last_verified: 2026-08-16
applies_to: "Tactus 0.3.0 and Segno 0.3.0"
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
stderr. On PowerShell, pass JSON params in single quotes.

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
Business-state history and Tactus journals can contain sensitive plugin output.

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

Use [Getting started](../getting-started.md) for installation and the
[troubleshooting guide](../troubleshooting.md) for failure diagnosis.
