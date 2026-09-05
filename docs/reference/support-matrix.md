---
title: Support matrix
status: alpha
owners: [release]
last_verified: 2026-09-05
applies_to: "Clef/Segno 0.3.0.0, Tactus 0.3.0, and Motivo Studio 0.3.0"
platforms: [windows, ubuntu]
---

# Support matrix

`Current gate` means the source workflow requires deterministic checks on
Windows and Ubuntu. It does not imply a packaged/signed distribution or a
successful call to a live provider account.

| Surface | Version/target | Windows | Ubuntu | Evidence boundary |
| --- | --- | --- | --- | --- |
| Repository license | `AGPL-3.0-only` | Current | Current | root `LICENSE`, Cargo workspace, both Cabal packages, and Motivo package metadata agree |
| Clef Haskell package | Cabal `0.3.0.0`, GHC2021, `base >=4.20 && <4.23` | Current gate | Current gate | `cabal build/test --builddir=Build/cabal`; fake JSONL plugins exercise typed tasks/plugins, norms/rubrics, bounded refinement, and incremental events |
| Tactus runtime/CLI | Rust crate `0.3.0`, stable Rust | Current gate | Current gate | format, package-scoped check/test/clippy; commands include workspace/workflow/plugin operations plus bounded `session list/show/answer` |
| Plugin process ABI | `agenstro.plugin/v1` | Current gate | Current gate | strict correlation/lifecycle, Unicode, malformed/oversized frames, immediate events, bounded transport, low-priority observation loss, authoritative terminal preservation, and exit behavior use local fakes |
| Run journal | `agenstro.trace/v1` | Current gate | Current gate | ordered append-flushed diagnostic events, atomic terminal summary, prompt/provider raw redaction, terminal-value summaries, and degraded-writer outcome preservation are tested; no replay/rollback claim |
| Process supervision | Unix process group / Windows Job Object | Current gate | Current gate | deadline/cancellation/protocol failure terminate the owned group; deliberate Unix session escape remains outside process-group containment; remote completion is unknowable |
| Codex adapter | Local `codex` executable | Adapter/fake tested | Adapter/fake tested | `describe`, offline smoke, dangerous-bypass argv, model/effort, and native output parsing; login/live behavior not a default gate |
| Claude Code adapter | Local `claude` executable; registry key `claude-code` | Adapter/fake tested | Adapter/fake tested | print/stream argv and dangerous permission bypass are fake-tested; login/live behavior not a default gate |
| OpenCode adapter | Local `opencode` executable | Adapter/fake tested | Adapter/fake tested | `--auto`, inline `permission=allow`, model/variant, and parsing are fake-tested; `full_bypass=false` because deny/managed policy may win |
| `workspace.paths` effect | Metadata, size, and SHA-256 snapshots | Current gate | Current gate | snapshot/diff/forget and observer calls; no reads, transient-write detection, content retention, attribution, CAS, or rollback |
| Generic `[plugins]` registry | Any one-shot executable | Current gate | Current gate | typed TOML/runtime JSON plus Clef `jsonPlugin`/`rawPlugin` and Tactus `plugin-call`; implementation language is unrestricted |
| Norm/checker boundary | `agenstro.norm/v1` over `agenstro.plugin/v1` | Current gate | Current gate | Haskell wire/judge tests and 24 Python fixtures cover catalogue records, external-wrapper routing, malformed/unsupported checks, strict JSON/correlation, exact terminal framing, and one-based inclusive loci |
| Tactus Studio control API | `tactus.control/v1` + `agenstro.studio/v1` | Current gate | Current gate | redacted inspect projection, four-category natural-language presentation, decimal-string counters, bounded run-event pages, run-id/path validation, and trace integrity use Rust tests |
| Tactus session control | `tactus.control/v1` + `agenstro.session/v1` | Current gate | Current gate | bounded list/show, static link/reparse refusal, typed document invariants, cross-process turn CAS, right-biased answers, atomic current-state replacement, and append-only answer evidence use Rust tests; hostile concurrent namespace replacement is outside the trusted-workspace model |
| `tactus smoke` | Offline unless `--live` | Current gate | Current gate | default sends no model prompt; CI uses fakes; live native/account compatibility is opt-in evidence |
| Topology-holes example | Four Haskell workflow stages + offline Rust oracle | Current gate | Current gate | real Tactus -> runghc -> Clef -> dispatch acceptance runs 010 -> 040 with parallel reviews and observer journals; the oracle verifies holes/Euler independently |
| Motivo Studio | TypeScript/Electron `0.3.0`, Node >=22.12 | Current gate | Current gate | format/lint/typecheck/Vitest/package; task and workspace views plus existing session answers use fake Tactus with no model credentials; packaged app requires external `tactus` |
| Motivo task method | `motivo.task/v1`, local IPC and `.motivo` records | Current gate | Current gate | fake reports/processes exercise call budgets, optional investigation branches, method override, pause, report validation, atomic history, and interrupted-outcome handling; no independent task-correctness or capability claim |
| Segno Flow | Cabal `0.3.0.0`, GHC2021, single-node driver | Current gate | Current gate | `cabal build/test --builddir=Build/cabal`; virtual time and fake process boundaries cover planning/execution without a model or wall-clock minute |
| Segno trigger composition | `Trigger state event` plus map/filter/merge/gate | Current gate | Current gate | GHC checks typed payload transformations and state-aware gates; plugin leaf manifests remain open JSON |
| Segno time plugins | `time.interval`, `time.cron` (UTC) | Current gate | Current gate | pure plan/poll tests cover cursors, due occurrences, and next wake; plugin processes never sleep |
| Segno SQLite state/lifecycle | business and lifecycle databases below `.tactus/segno/state` | Current gate | Current gate | local tests cover cross-job occurrence identity, checkpoint scoping, stale fences, trigger-failure isolation, and unknown non-retry; single-node only |
| Active-window plugin | Built-in Haskell plugin | Current gate | Structured unsupported result | Windows uses the Win32 package; CI type-checks and fake-tests minute scheduling but does not collect a developer's real foreground title |
| Historical worker/daemon/cell paths | Legacy evidence only | Not gated | Not gated | not installable through current Tactus and not compatible state |

## CLI and network behavior

| Command or UI action | Offline by default | Can execute arbitrary trusted code | Can contact a provider |
| --- | --- | --- | --- |
| `init`, `list`, `prompt`, `doctor`, `studio inspect/events`, `runs list/summarize/unfinished/show`, `session list/show` | Yes | Plugin commands may be inspected, not invoked by these queries | No |
| `runs archive/gc` | Yes | Moves or deletes only validated eligible local journals; dry-run unless `--yes` | No |
| `session answer` | Yes | Updates one local typed session and its answer transcript | No |
| `check` | Yes apart from package resolution | Runs Cabal/GHC | No model call |
| `run` | Depends on script | Yes, ordinary Haskell `IO` | Yes if the script invokes a provider/plugin |
| `smoke` | Yes | Starts selected plugin executable | Only with `--live` for provider adapters |
| `plugin-call` | Depends on method | Yes | Yes for provider/network plugins |
| `generate` | No | Provider may edit the workspace | Yes |
| Motivo Tasks: create/list/show | Yes | Reads/writes Motivo task records | No |
| Motivo Tasks: continue | No | Native agent may edit, run project checks, or create/invoke project plugins through Tactus | Yes; lead and investigators share the call budget |
| `segno init/list/status/history` | Yes | Local layout and SQLite inspection | No |
| `segno install` | No provider call | Runs the trusted task in describe mode through Tactus | Only if the task violates the describe contract with direct `IO` |
| `segno once/driver` | Depends on installed tasks | Yes; executes each due Clef task through Tactus | Yes if a task invokes a provider/network plugin |

## Timing and delivery contract

| Control or outcome | Supported contract |
| --- | --- |
| Direct `tactus check/run --timeout-seconds` | Explicit option overrides workspace policy; otherwise check defaults to 1,800 seconds and each complete workflow script has a 15,300-second outer deadline; `0` explicitly disables the direct deadline |
| `segno install/once/driver --task-timeout-seconds` | 15,300-second default for each Tactus build/run phase; accepted range 1 through 604,800; `0` is rejected |
| Provider dispatch | Defaults nest native CLI 13,440 seconds, Tactus dispatch 13,500 seconds, and Clef outer supervision 14,400 seconds below the workflow script's 15,300-second outer deadline; workspace `limits` also bound stdout/frame/result retention and provider concurrency |
| `segno driver --poll-seconds` | Positive finite maximum idle wait; default 1 second; it is neither the trigger interval nor a task deadline |
| Observation delivery | Bounded and non-authoritative; provider/UI layers may aggregate or coalesce progress, while Tactus drops excess low-priority callbacks into `events_dropped`; callback degradation becomes `observation_error`, and neither changes the authoritative invocation terminal |
| Motivo action-output projection | A byte/frame overrun emits one `[warning]`, discards later raw projection, and continues draining Tactus without changing the child outcome |
| Motivo task call budget | 1 through 20 native agent episodes per continuation, default 4; includes investigators and reserves lead integration capacity; not a token/cost/time guarantee |
| Motivo task pause and interruption | Pause waits for current calls; budget exhaustion saves a handoff; interruption or an unusable post-execution report is not retried automatically, and unknown continuation requires a user note |
| Occurrence delivery | At least once; tasks should deduplicate external effects with the occurrence idempotency key |
| Ambiguous execution | `OutcomeUnknown` is terminal and not automatically retried; successful checkpoints remain durable and require explicit external reconciliation |

## Trust and capability boundaries

- Haskell workflows, configured plugins, and native provider CLIs inherit the
  caller's environment, credentials, workspace, network, and user permissions.
- Built-in Codex/Claude adapters deliberately use dangerous approval/sandbox
  bypass flags. OpenCode has the documented weaker caveat.
- Protocol validation, argv execution, process groups, deadlines, and resource
  bounds improve reliability; they do not authenticate or sandbox code.
- Tactus has no daemon, login/auth layer, credential broker, general artifact
  tracker, workflow checkpoint, or rollback. Session answers use a narrow local
  turn CAS; it is not a workspace transaction facility.
- Segno adds a local long-lived scheduling loop and business-state CAS. It is
  not a network daemon, auth service, artifact store, workspace transaction, or
  external-effect rollback mechanism.
- Motivo owns task-level method and `.motivo` records, using existing Tactus
  dispatch for provider calls. It does not add a Tactus domain loop, change
  `tactus.control/v1`, write Tactus sessions, or change Segno scheduling.
- Motivo investigator prompts ask for read-only work in a shared environment.
  They do not enforce filesystem restrictions or isolate concurrent writes.
  A task's `completed` and check reports are agent claims, not certified results.
- Motivo task history contains goals, constraints, notes, and reports. It is
  separate from redacted Tactus journals and is not a workspace backup or replay.
- `workspace.paths` is final-state evidence only. Direct `IO`, reads, transient
  writes, and concurrent processes can fall outside or blur its evidence.
- Run journals redact prompt/provider raw fields and summarize terminal values
  and native stderr before persistence. Bounded errors, hashes, and path
  metadata can still be sensitive.
- The active-window example is model/network-free, but window titles can expose
  documents, URLs, and account names. Titles remain in Segno SQLite business
  history until the user manages that state; Tactus retains only a redacted
  diagnostic summary of the plugin result.

## Development tool boundary

The current runtime installation requires Rust plus Cabal/GHC. Segno is built
and installed with Cabal. On Windows, the documented source install places
both `tactus.exe` and `segno.exe` in `%USERPROFILE%\.cargo\bin`; GHCup normally
provides GHC/Cabal through `C:\ghcup\bin`. The first Cabal build can fetch and
compile dependencies for several minutes, while later checks reuse the Cabal
store and build cache. Motivo development additionally requires Node.js 22.12
or newer. Python is not a runtime dependency of Tactus, Segno, or Motivo; it is
used only when a Python plugin such as the optional reference norm checker is
selected, or when MkDocs is built.

The checked-in Cargo configuration directs Rust gate output to ignored
`Build/cargo`. Warm validation retains that rebuildable tree; use the local
quality `Clean` profile or its size threshold when disk use matters. Cabal,
MkDocs, and Electron Forge likewise use dedicated subdirectories below
`Build/`.

## Version combination

The supported source combination is Clef `0.3.0.0`, Segno `0.3.0.0`,
Tactus `0.3.0`, and Motivo Studio `0.3.0`. Motivo Tasks add a local application
contract without changing the existing Tactus Studio/session wires. There is no
compatibility claim for mixing this with historical workers, daemon state,
the old Motivo gRPC/PTY ownership, or the removed
Python/Rust Segno registry and database. See the [migration
guide](../migrations/0.2-to-haskell-0.3.md) for side-by-side context.
