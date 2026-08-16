---
title: Support matrix
status: alpha
owners: [release]
last_verified: 2026-08-15
applies_to: "Clef 0.3.0.0 + Tactus 0.3.0"
platforms: [windows, ubuntu]
---

# Support matrix

`Current gate` means the source workflow requires deterministic checks on
Windows and Ubuntu. It does not imply a packaged/signed distribution or a
successful call to a live provider account.

| Surface | Version/target | Windows | Ubuntu | Evidence boundary |
| --- | --- | --- | --- | --- |
| Clef Haskell package | Cabal `0.3.0.0`, GHC2021, `base >=4.20 && <4.23` | Current gate | Current gate | `cabal build all` and `cabal test all`; fake JSONL plugins exercise typed tasks/plugins and incremental events |
| Tactus runtime/CLI | Rust crate `0.3.0`, stable Rust | Current gate | Current gate | format, package-scoped check/test/clippy; commands are `init`, `list`, `prompt`, `generate`, `check`, `run`, `doctor`, `smoke`, and `plugin-call` |
| Plugin process ABI | `agenstro.plugin/v1` | Current gate | Current gate | strict correlation/lifecycle, Unicode, malformed/oversized frames, immediate events, bounded transport, and exit behavior use local fakes |
| Run journal | `agenstro.trace/v1` | Current gate | Current gate | ordered append-flushed events and atomic terminal summary are tested; no replay/rollback claim |
| Process supervision | Unix process group / Windows Job Object | Current gate | Current gate | deadline/cancellation/protocol failure terminate the owned group; deliberate Unix session escape remains outside process-group containment; remote completion is unknowable |
| Codex adapter | Local `codex` executable | Adapter/fake tested | Adapter/fake tested | `describe`, offline smoke, dangerous-bypass argv, model/effort, and native output parsing; login/live behavior not a default gate |
| Claude Code adapter | Local `claude` executable; registry key `claude-code` | Adapter/fake tested | Adapter/fake tested | print/stream argv and dangerous permission bypass are fake-tested; login/live behavior not a default gate |
| OpenCode adapter | Local `opencode` executable | Adapter/fake tested | Adapter/fake tested | `--auto`, inline `permission=allow`, model/variant, and parsing are fake-tested; `full_bypass=false` because deny/managed policy may win |
| `workspace.paths` effect | Metadata, size, and SHA-256 snapshots | Current gate | Current gate | snapshot/diff/forget and observer calls; no reads, transient-write detection, content retention, attribution, CAS, or rollback |
| Generic `[plugins]` registry | Any one-shot executable | Current gate | Current gate | typed TOML/runtime JSON plus Clef `jsonPlugin`/`rawPlugin` and Tactus `plugin-call`; implementation language is unrestricted |
| Tactus Studio control API | `tactus.control/v1` + `agenstro.studio/v1` | Current gate | Current gate | redacted inspect projection, decimal-string counters, bounded run-event pages, run-id/path validation, and trace integrity use Rust tests |
| `tactus smoke` | Offline unless `--live` | Current gate | Current gate | default sends no model prompt; CI uses fakes; live native/account compatibility is opt-in evidence |
| Topology-holes example | Four Haskell workflow stages + offline Rust oracle | Current gate | Current gate | real Tactus -> runghc -> Clef -> dispatch acceptance runs 010 -> 040 with parallel reviews and observer journals; the oracle verifies holes/Euler independently |
| Motivo Studio | TypeScript/Electron `0.3.0`, Node >=22.12 | Current gate | Current gate | format/lint/typecheck/Vitest/package; fake Tactus only, no model credentials; packaged app requires external `tactus` |
| Segno Flow | Frozen | Not gated | Not gated | source retained; scheduling and replay are outside `0.3` |
| Historical worker/daemon/cell paths | Legacy evidence only | Not gated | Not gated | not installable through current Tactus and not compatible state |

## CLI and network behavior

| Command | Offline by default | Can execute arbitrary trusted code | Can contact a provider |
| --- | --- | --- | --- |
| `init`, `list`, `prompt`, `doctor`, `studio inspect/events` | Yes | Plugin commands may be inspected, not invoked by these queries | No |
| `check` | Yes apart from package resolution | Runs Cabal/GHC | No model call |
| `run` | Depends on script | Yes, ordinary Haskell `IO` | Yes if the script invokes a provider/plugin |
| `smoke` | Yes | Starts selected plugin executable | Only with `--live` for provider adapters |
| `plugin-call` | Depends on method | Yes | Yes for provider/network plugins |
| `generate` | No | Provider may edit the workspace | Yes |

## Trust and capability boundaries

- Haskell workflows, configured plugins, and native provider CLIs inherit the
  caller's environment, credentials, workspace, network, and user permissions.
- Built-in Codex/Claude adapters deliberately use dangerous approval/sandbox
  bypass flags. OpenCode has the documented weaker caveat.
- Protocol validation, argv execution, process groups, deadlines, and resource
  bounds improve reliability; they do not authenticate or sandbox code.
- Tactus has no daemon, login/auth layer, credential broker, CAS/artifact
  tracker, checkpoint, or rollback.
- `workspace.paths` is final-state evidence only. Direct `IO`, reads, transient
  writes, and concurrent processes can fall outside or blur its evidence.
- Run journals can contain sensitive provider output and are not automatically
  redacted.

## Development tool boundary

The current runtime installation requires Rust plus Cabal/GHC. Motivo
development additionally requires Node.js 22.12 or newer. Python is not a
runtime dependency of Tactus or Motivo; it is used only if a third-party plugin
chooses it or when MkDocs is built.

Rust gate commands must use a system-temporary `CARGO_TARGET_DIR`, and validation
must finish with `cargo clean --target-dir <that-exact-path>` to avoid a large
repository-local build tree.

## Version combination

The supported source combination is Clef `0.3.0.0` with Tactus `0.3.0`. There
is no compatibility claim for mixing it with historical workers, daemon state,
the old Motivo daemon/runtime ownership, or Segno scheduling state. See the
[migration guide](../migrations/0.2-to-haskell-0.3.md) for side-by-side context.
