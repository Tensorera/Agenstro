# Agenstro 0.3 Haskell-first Documentation

This site describes the current `0.3` source release path: Clef is a typed
Haskell EDSL, and Tactus is the Python 3.12 project-local CLI that prepares,
checks, and runs ordinary Haskell workflow programs.

## Current release contract

| Surface | Contract |
| --- | --- |
| Clef | Cabal package `0.3.0.0`; GHC2021; `base >=4.20 && <4.23` |
| Tactus | Python package `0.3.0`; `init`, `list`, `prompt`, `generate`, `check`, `run`, `doctor`, and `smoke` |
| Providers | Replaceable one-shot JSONL plugins for Codex, Claude Code, and OpenCode |
| Effect | `workspace.paths` observes path snapshots and differences; it does not sandbox or roll back changes |
| Platforms | Windows and Ubuntu are required jobs in the Haskell/Tactus CI matrix |

From the repository root, the source gates are:

```powershell
cabal build all
cabal test all --test-show-details=direct

python -m pip install -e "./tactus-runtime[dev]"
python -m pytest tactus-runtime/tests
python -m ruff check tactus-runtime/src/tactus_runtime tactus-runtime/tests
python -m ruff format --check tactus-runtime/src/tactus_runtime tactus-runtime/tests
python -m pyright -p tactus-runtime/pyproject.toml
```

Those tests use local fakes and do not contact a model provider.

## Tactus safety boundary

`tactus check` performs Cabal/GHC compile checks. `tactus run` executes trusted
Haskell programs, and `tactus generate` invokes the selected coding-agent
plugin. Provider and effect commands inherit the user's environment and run
without a shell, but they are still arbitrary local executables rather than a
security boundary.

`tactus smoke` is offline by default: it performs executable/version health
checks but sends no model prompt. A real provider request happens only when
`--live` is supplied. OpenCode is supported at the JSONL adapter boundary, but
its `--auto` mode cannot prove a full approval bypass in the presence of
explicit deny or managed configuration.

## Start here

| Goal | Page |
| --- | --- |
| Install Tactus and run the offline path | [Getting started](getting-started.md) |
| Understand ownership and execution flow | [Architecture](architecture.md) |
| Check current platform and component status | [Support matrix](reference/support-matrix.md) |
| Inspect the provider/effect wire contract | [Local plugin protocol v1](reference/plugin-protocol-v1.md) |
| Diagnose toolchain, encoding, or plugin failures | [Troubleshooting](troubleshooting.md) |
| See what is current, exploratory, or frozen | [Roadmap](roadmap.md) |
| Understand the Haskell and trusted-plugin decision | [ADR-0003](adr/0003-haskell-dsl-and-local-plugins.md) |
| Move a `0.2` workspace to the new path | [0.2 to Haskell 0.3 migration](migrations/0.2-to-haskell-0.3.md) |

Motivo Studio and Segno Flow are frozen for this release. The old `0.2`
Rust/Python code and documentation remain in Git as legacy evidence while the
cutover is completed; they are not current release claims or gates.
