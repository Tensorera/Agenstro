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
| Tactus runtime/CLI | Rust crate `0.3.0`, stable Rust | Current gate | Current gate | Cargo workspace format/check/test/clippy; isolated script-root selection and idempotent skill/template installation are covered alongside workspace/workflow/plugin and session operations |
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
| Motivo methods | Eight Haskell templates and project skill | Current gate | Current gate | `.tactus/motivoscript` stays separate from business scripts; input records, samples, artifacts, and reports contain evidence for the existing coding agent, not a certified task result |
| Motivo Studio | Self-contained HTML `0.3.0` | Current gate | Current gate | format/lint/reader tests and static build; core content is pre-rendered, with no runtime Node/Electron, IPC, server, or execution controls |
| `motivo.test` effect | Rust `0.3.0`, Linux Bubblewrap backend | Execution refused | Requires usable Bubblewrap namespaces | Real isolation tests cover confined persistent writes, project reads, child-process containment, shared deadlines, bounded logs, and failure without host execution; other platforms refuse experiments |
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
| Motivo method with existing notes | Yes apart from Haskell dependency resolution | Runs the selected Haskell template and publishes method artifacts | No implicit provider call |
| Motivo method with an explicit provider invocation | No | Executes the configured trusted provider through Clef/Tactus | Yes; the main agent selects the existing registry entry |
| Motivo `probe` / `motivo.test run` | Yes | Runs experiment code within the plugin's supported Linux isolation boundary | No network in the experiment |
| Open or refresh a Motivo HTML report | Yes | Reading controls only; no local execution request | No |
| `segno init/list/status/history` | Yes | Local layout and SQLite inspection | No |
| `segno install` | No provider call | Runs the trusted task in describe mode through Tactus | Only if the task violates the describe contract with direct `IO` |
| `segno once/driver` | Depends on installed tasks | Yes; executes each due Clef task through Tactus | Yes if a task invokes a provider/network plugin |

## Timing and delivery contract

| Control or outcome | Supported contract |
| --- | --- |
| Direct `tactus check/run --timeout-seconds` | Explicit option overrides workspace policy; otherwise check processes default to 1,800 seconds and run preparation/execution processes to 15,300 seconds each; these are per-process deadlines, not one batch budget; `0` disables the direct deadline |
| `segno install/once/driver --task-timeout-seconds` | 15,300-second default for each Tactus build/run phase; accepted range 1 through 604,800; `0` is rejected |
| Provider dispatch | Defaults nest native CLI 13,440 seconds, Tactus dispatch 13,500 seconds, and Clef outer supervision 14,400 seconds below the workflow script's 15,300-second outer deadline; workspace `limits` also bound stdout/frame/result retention and provider concurrency |
| `segno driver --poll-seconds` | Positive finite maximum idle wait; default 1 second; it is neither the trigger interval nor a task deadline |
| Observation delivery | Bounded and non-authoritative; provider/UI layers may aggregate or coalesce progress, while Tactus drops excess low-priority callbacks into `events_dropped`; callback degradation becomes `observation_error`, and neither changes the authoritative invocation terminal |
| Motivo experiment deadline | One monotonic deadline covers experiment preparation and execution; default/maximum 600 seconds, accepted range 1–600; termination time is reserved within that budget |
| Motivo HTML publication | Complete file snapshots; refresh reads the newer file, not a live agent stream; generation time and last-recorded state may become stale |
| Motivo method result | `recorded` says notes were published, not that the task is correct; partial or interrupted experiment evidence requires inspection rather than automatic replay |
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
- The user's chosen coding agent owns the conversation and method selection.
  Motivo supplies skill guidance, editable Haskell templates, and
  `.tactus/motivo` evidence. It does not own a provider registry, task loop,
  Tactus sessions, or Segno scheduling.
- `.tactus/motivoscript` separates method sources from business scripts, but
  selecting a source directory does not sandbox arbitrary Haskell or provider
  calls. Investigations in separate contexts do not imply isolated writers.
- The Linux `motivo.test` backend confines persistent experiment writes to
  `.tactus/motivotest/<run>/<sample>` and provides read-only project access.
  Unsupported platforms or unavailable isolation refuse execution; no fallback
  runs the same command with host privileges. This is a plugin-specific
  boundary rather than a universal guarantee for workflow code.
- Method artifacts contain user goals, notes, sources, and experiment output.
  They are distinct from redacted Tactus journals and are not backups or replay
  state. A finished report or an agent's check claim does not prove correctness.
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
store and build cache. Motivo's HTML development checks require Node.js 22.12
or newer; opening generated reports and running the Haskell methods require
neither Node nor Electron. Experiments additionally require the `motivo-test`
executable and its usable Linux Bubblewrap backend. Python is not a runtime
dependency of Tactus, Segno, or Motivo; it is
used when a Python plugin such as the optional reference norm checker is
selected, when MkDocs is built, or for offline Motivo acceptance tests.

The checked-in Cargo configuration directs Rust gate output to ignored
`Build/cargo`. Warm validation retains that rebuildable tree; use the local
quality `Clean` profile or its size threshold when disk use matters. Cabal,
MkDocs, and Motivo's static build likewise use dedicated subdirectories below
`Build/`.

## Version combination

The supported source combination is Clef `0.3.0.0`, Segno `0.3.0.0`,
Tactus `0.3.0`, Motivo Studio `0.3.0`, and the `motivo-test` plugin `0.3.0`.
The Motivo skill and Haskell methods replace the desktop task loop under
[ADR-0008](../adr/0008-agent-led-motivo.md), preserving Tactus Studio/session
APIs. Old `.motivo/tasks` records are not automatically resumed or converted.
There is no
compatibility claim for mixing this with historical workers, daemon state,
the old Motivo gRPC/PTY ownership, or the removed
Python/Rust Segno registry and database. See the [migration
guide](../migrations/0.2-to-haskell-0.3.md) for side-by-side context.
