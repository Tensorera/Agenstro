---
title: Agenstro CLI reference
status: alpha
owners: [tactus]
last_verified: 2026-09-05
applies_to: "Tactus/Motivo Studio 0.3.0 and Segno 0.3.0"
platforms: [windows, ubuntu]
---

# Agenstro CLI reference

This is the supported human-facing command surface for the 0.3 source alpha.
The Tactus script-selection options were checked against compiled `--help`
output on 2026-09-05. Internal
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
| `tactus list` | List ordered entries and helpers | `--root`, `--scripts-dir PATH`, `--json` |
| `tactus prompt` | Print resolved generation instructions | `--root` |
| `tactus doctor` | Validate config, GHC/Cabal, SDK, and plugin commands | `--root`, `--json` |
| `tactus runtime-json` | Print Clef's normalized runtime config | `--root` |
| `tactus check [SCRIPT...]` | Compile-check an explicit selection without executing | `--scripts-dir PATH`, `--all`, `--from NNN`, `--through NNN`, `--package NAME`, `--keep-going`, `--timeout-seconds` |
| `tactus run` | Execute an explicit selection of ordered entry scripts | `--scripts-dir PATH`, repeatable `--script`, `--all`, `--from NNN`, `--through NNN`, `--package NAME`, `--keep-going`, `--timeout-seconds`, `-- ARG...` |
| `tactus generate GOAL...` | Ask one provider to create or update Haskell sources, including helpers | `--provider NAME`, `--timeout-seconds`, `--json` |
| `tactus plugin-call NAME METHOD` | Invoke a registry entry directly | `--namespace`, `--params JSON`, `--timeout-seconds`, `--json` |
| `tactus smoke [NAME...]` | Probe configured plugins | `--live`, `--json` |
| `tactus runs list` | List completed outcomes plus valid `open` and protected `corrupt` journals, newest first | `--state`, `--since`, `--limit`, `--json` |
| `tactus runs summarize` | Aggregate run states and invocation kinds | `--state`, `--since`, `--json` |
| `tactus runs unfinished` | List valid `open` journals only; use `list --state corrupt` for malformed evidence | `--since`, `--limit`, `--json` |
| `tactus runs show RUN_ID` | Read one bounded durable-event page | `--after`, `--limit`, `--max-bytes`, `--json` |
| `tactus runs archive` | Preview or move eligible old journals | required `--before`; `--yes` applies |
| `tactus runs gc` | Preview or delete eligible archived journals | optional `--before`; `--yes` applies |
| `tactus studio inspect` | Return a redacted Studio workspace projection | `--exact-root`, `--run-limit` |
| `tactus studio events RUN_ID` | Read one bounded trace page | `--after`, `--limit`, `--max-bytes` |
| `tactus session list` | Return newest validated session views | `--root`, `--limit` |
| `tactus session show` | Return one session view | `--root`, `--session ID` |
| `tactus session answer` | Compare-and-set one pending choice | `--root`, `--session ID`, `--turn`, `--axis`, `--option`, `--note` |

Without `--json`, the Tactus-generated user-log vocabulary is closed to
`[state]`, `[info]`, `[warning]`, and `[error]`, each followed by bounded
natural-language text. Native stderr, provider JSON/free text, event payloads,
stable codes, and counters are technical diagnostics and are never promoted
directly into that layer. `check`/`run` may attach compiler or workflow process
output separately. Machine-mode JSON remains structured by design.

`list`, `check` and `run` default to `.tactus/scripts`. Their generic
`--scripts-dir PATH` selects one source directory below `.tactus`, relative to
the workspace root (or supplied as a contained absolute path). Discovery,
explicit source validation and helper imports use that selected root. The
parameter changes source selection, not runtime filesystem permissions.
Default business `--all` never includes `.tactus/motivoscript`.

`check`/`run --timeout-seconds` applies separately to each Cabal/GHC/runghc
process; zero disables this existing per-process timeout. It is not the total
budget of a multi-command experiment. Motivo's separate `motivo.test` plugin
enforces its own positive deadline of at most 600 seconds.

Clef is always exposed to `check` and `run`. Repeat `--package` for extensions,
for example Segno:

```powershell
tactus check --root D:\work\project --package segno-flow `
  .tactus\scripts\900_record_active_window.hs

tactus run --root D:\work\project --package segno-flow `
  --script .tactus\scripts\900_record_active_window.hs
```

`check` and `run` never infer “all” from an empty selection. Supply one or more
paths, `--all`, or an inclusive `--from` / `--through` range. Archive and GC
are dry-run previews unless `--yes` is present; open, corrupt, and unresolved
`outcome_unknown` journals are always protected. JSON from `runs show` exposes
the journal's validated durable diagnostic data; treat it as potentially
sensitive even though reads are byte- and event-bounded.

`generate` and `smoke --live` may contact or bill a provider. Plain `smoke` is
an offline executable/capability probe. `check` does not execute workflow code;
`run` executes trusted Haskell and may perform arbitrary `IO`.

Registry names can collide. Use an explicit namespace when needed:

```powershell
tactus plugin-call workspace.paths describe --namespace effect --params '{}'
tactus plugin-call calculator add --namespace plugin `
  --params '{"left":19,"right":23}' --json
```

Session commands always emit one `tactus.control/v1` JSON envelope. An answer
is accepted only while the session awaits an answer and while its turn, axis,
and option still match the pending brief:

```powershell
tactus session list --root D:\work\project --limit 50
tactus session show --root D:\work\project --session session-7f3a91
tactus session answer --root D:\work\project `
  --session session-7f3a91 --turn 3 --axis desk.frame --option fixed `
  --note "Prefer parts that remain repairable"
```

`session_turn_stale` is not retryable: refetch the current session before
making another decision. This stage intentionally has no `session advance`
command because planner registration and execution are not yet specified. See
the [session control API](session-control-v1.md).

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

The task timeout is per Tactus build/run phase, defaults to 15,300 seconds, and
accepts 1–604,800 seconds. Segno derives a longer Running lease from it.

`history --state-key` and `history --occurrence` are mutually exclusive.
Business-state history can contain plugin values. Tactus journals persist only
a bounded diagnostic projection: prompt/provider raw text, terminal success
values, and native stderr are redacted or summarized, but errors, hashes, and
path metadata can still be sensitive.

## Motivo methods and offline Studio reports

The entrypoint is the user's coding agent. Initialize the project, open Claude
Code, Codex or another CLI there, then ask it to use the installed Motivo skill.
There is no current Electron launcher, provider picker or desktop task service.

Motivo provides eight independent templates: clarify, investigate, analyze,
research, probe, organize, retrospect and handoff. Their numbering does not
require execution as a pipeline. By default a method records Markdown prepared
by the main agent without calling another model:

```powershell
tactus run --root D:\work\project --scripts-dir .tactus/motivoscript `
  --script .tactus/motivoscript/020_investigate.hs `
  -- --input .tactus/motivo/drafts/question.md --agent "Codex"
```

Non-probe templates also accept an explicit `--provider NAME` and optional
`--model MODEL` for one independent context. This uses `invokeWith` and never
falls back to an implicit default provider. Requested model identity is distinct
from actual identity reported by a provider. These are method arguments after
`--`, not new Tactus runtime flags.

`050_probe.hs` accepts `--run-id`, `--sample-id`, and
`--timeout-seconds 1..600`, then a second `--` followed by experiment argv. It
calls only the registered `motivo.test` effect and rejects `--provider`. Prepare
fixtures in `.tactus/motivotest/<run>/<sample>`; supported Linux isolation
exposes those fixtures at writable `/work` and the project at read-only
`/project`. Other methods are available when an enforcing experiment backend is
unavailable; experiments must not fall back to unrestricted execution.

Each run writes `.tactus/motivo/runs/<run-id>/report.html` and updates
`.tactus/motivo/index.html`. Open either file in a browser. For example, after
recording a method on Windows:

```powershell
Start-Process 'D:\work\project\.tactus\motivo\index.html'
```

The HTML is generated by Haskell and contains the snapshot's data. It needs no
server or port, and cannot invoke providers, answer sessions or execute scripts.
Refresh the page to read a newer generated snapshot. See
[Motivo Studio](../motivo-studio.md), the installed Motivo skill, and
[ADR-0008](../adr/0008-agent-led-motivo.md).

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
