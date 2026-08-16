---
title: Support matrix
status: alpha
owners: [release]
last_verified: 2026-08-15
applies_to: "0.3.0"
platforms: [windows, ubuntu]
---

# Support Matrix

`Current gate` means the dedicated Haskell/Tactus workflow requires the check
on both `windows-latest` and `ubuntu-latest`. It does not imply a packaged,
signed distribution or a successful live provider call.

| Surface | Version/target | Windows | Ubuntu | Status and evidence |
| --- | --- | --- | --- | --- |
| Clef Haskell package | Cabal `0.3.0.0`, GHC2021, `base >=4.20 && <4.23` | Current gate | Current gate | `cabal build all` and `cabal test all`; tests use a fake JSONL plugin executable |
| Tactus CLI | Python `0.3.0`, CPython 3.12 | Current gate | Current gate | `pytest`, Ruff lint/format, and strict Pyright; commands are `init`, `list`, `prompt`, `generate`, `check`, `run`, `doctor`, and `smoke` |
| JSONL plugin ABI | `agenstro.plugin/v1` | Current gate | Current gate | Correlation, event, terminal-result, malformed-output, and subprocess-exit behavior are tested with local fakes |
| Codex adapter | Local `codex` executable | Adapter/fake tested | Adapter/fake tested | `describe`, offline `smoke`, and invocation argv/output parsing are covered; authentication and live model behavior are not CI gates |
| Claude Code adapter | Local `claude` executable | Adapter/fake tested | Adapter/fake tested | Canonical name `claude-code`; authentication and live model behavior are not CI gates |
| OpenCode adapter | Local `opencode` executable | Adapter/fake tested | Adapter/fake tested | Full approval bypass cannot be proven: `--auto` and inline `permission=allow` do not override explicit deny or managed configuration |
| `workspace.paths` effect | Path metadata and SHA-256 content observations | Current gate | Current gate | Snapshot/diff/forget and observer calls are tested; reads, transient writes, authorization, rollback, and sandboxing are not provided |
| `tactus smoke` | Offline unless `--live` | Version probe only | Version probe only | The default request sets `live=false`; CI uses fakes and never performs an authenticated model request |
| Motivo Studio | Frozen | Not gated | Not gated | Still present in the source tree; no `0.3` support or packaging claim |
| Segno Flow | Frozen | Not gated | Not gated | Still present in the source tree; scheduling and replay are outside the `0.3` release |
| Clef/Tactus Python `0.2` archives | Legacy evidence | Not gated | Not gated | Removed Python surfaces are isolated under `archive/`; the Rust product cores were removed after snapshot `c679f45` |

## Trust and capability boundaries

- Haskell workflows, configured plugins, and provider CLIs run as trusted user
  code and inherit the caller's environment and credentials.
- The JSONL process boundary validates message shape and lifecycle. It does not
  authenticate a plugin or constrain its filesystem and network access.
- `workspace.paths` is observation only. Direct Haskell `IO`, direct commands,
  concurrent writers, reads, and transient writes can fall outside its
  evidence.
- `tactus generate`, `tactus smoke --live`, and some workflows started by
  `tactus run` can make live provider calls. The release CI runs none of them.

## Version combination

The supported source combination is Clef `0.3.0.0` with Tactus `0.3.0`. There
is no compatibility claim for mixing this path with archived `0.2` workers,
Motivo runtime, or Segno scheduler. Use the
[migration guide](../migrations/0.2-to-haskell-0.3.md) for a side-by-side
cutover and rollback plan.
