---
title: Source contracts
status: alpha
owners: [foundation]
last_verified: 2026-08-01
applies_to: "0.2.0"
platforms: [windows, ubuntu]
---

# Source Contracts

Use these sources rather than prose copies when exact fields or behavior
matter. Links target the repository's `main` branch; use the matching release
commit when auditing another version.

## Protocol

| Contract | Source |
| --- | --- |
| Product names, three daemon kinds, health, server versions and capabilities | [`proto/agentro/system/v1/system_service.proto`](https://github.com/Tensorera/agenstro/blob/main/proto/agentro/system/v1/system_service.proto) |
| Error domain, machine code, retry advice and trace ID | [`proto/agentro/common/v1/error.proto`](https://github.com/Tensorera/agenstro/blob/main/proto/agentro/common/v1/error.proto) |
| Capability stability | [`proto/agentro/common/v1/capability.proto`](https://github.com/Tensorera/agenstro/blob/main/proto/agentro/common/v1/capability.proto) |
| Bounded pagination | [`proto/agentro/common/v1/pagination.proto`](https://github.com/Tensorera/agenstro/blob/main/proto/agentro/common/v1/pagination.proto) |
| Workflow, execution, workspace and schedule RPC DTOs | [`proto/agentro`](https://github.com/Tensorera/agenstro/tree/main/proto/agentro) |

Generated Rust and TypeScript are derived artifacts. `proto/` is the only IDL
source tree.

## Configuration and Limits

There is no public `config.toml` schema or `doctor` configuration command in
this alpha. Configuration is currently constructor-level:

| Owner | Source |
| --- | --- |
| Historical Tactus admission, output, source, lease and shutdown bounds | [`daemon.rs` at the final Rust snapshot](https://github.com/Tensorera/agenstro/blob/c679f45b995228b675ef2f1221a16a9026604085/tactus-runtime/rust/tactus-core/src/daemon.rs) |
| Historical Tactus scan, path, object, manifest and restore bounds | [`checkpoint.rs` at the final Rust snapshot](https://github.com/Tensorera/agenstro/blob/c679f45b995228b675ef2f1221a16a9026604085/tactus-runtime/rust/tactus-core/src/checkpoint.rs) |
| Historical Tactus worker framing and output chunks | [`worker.rs` at the final Rust snapshot](https://github.com/Tensorera/agenstro/blob/c679f45b995228b675ef2f1221a16a9026604085/tactus-runtime/rust/tactus-core/src/worker.rs) |
| Segno dispatch, lease, tick and misfire bounds | [`segno-flow/rust/segnod/src/service.rs`](https://github.com/Tensorera/agenstro/blob/main/segno-flow/rust/segnod/src/service.rs) |
| Segno package authoring budgets | [`segno-flow/src/segno_flow/package.py`](https://github.com/Tensorera/agenstro/blob/main/segno-flow/src/segno_flow/package.py) |
| Electron bootstrap timeout/frame/token rules | [`motivo-studio/src/main/daemon/bootstrap.ts`](https://github.com/Tensorera/agenstro/blob/main/motivo-studio/src/main/daemon/bootstrap.ts) |

## Errors

Stable machine codes are uppercase, underscore-separated, and at most 64 bytes
at the shared Rust boundary. Display messages are not automation contracts.

| Surface | Source |
| --- | --- |
| Shared Rust error grammar | [`crates/agentro-contracts/src/error.rs`](https://github.com/Tensorera/agenstro/blob/main/crates/agentro-contracts/src/error.rs) |
| Clef RPC-to-exception mapping | [`archive/clef-sdk-python-0.2/src/clef_sdk/errors.py`](https://github.com/Tensorera/agenstro/blob/main/archive/clef-sdk-python-0.2/src/clef_sdk/errors.py) |
| Historical Tactus Rust application failures | [`daemon.rs` at the final Rust snapshot](https://github.com/Tensorera/agenstro/blob/c679f45b995228b675ef2f1221a16a9026604085/tactus-runtime/rust/tactus-core/src/daemon.rs) |
| Segno Python RPC mapping | [`segno-flow/src/segno_flow/client.py`](https://github.com/Tensorera/agenstro/blob/main/segno-flow/src/segno_flow/client.py) |
| Motivo sanitized renderer errors | [`motivo-studio/src/main/errors.ts`](https://github.com/Tensorera/agenstro/blob/main/motivo-studio/src/main/errors.ts) |

## CLI

| Entry | Implemented commands | Source |
| --- | --- | --- |
| legacy `tactus` | `script-check`, `script-run`, `studio`, `--version` | [archived Tactus 0.2 surface](https://github.com/Tensorera/agenstro/blob/main/archive/tactus-runtime-0.2/README.md) |
| `motivo-studio` Python compatibility entry | Migration message only | [`archive/tactus-runtime-0.2/src/tactus_runtime/studio.py`](https://github.com/Tensorera/agenstro/blob/main/archive/tactus-runtime-0.2/src/tactus_runtime/studio.py) |
| `segno-flow` | offline `package build`, report readers, thin daemon calls, `--version` | [`segno_flow/cli.py`](https://github.com/Tensorera/agenstro/blob/main/segno-flow/src/segno_flow/cli.py) |
| `segno-flow-ui` | Replaces itself with `motivo-studio --surface scheduler` | [`segno_flow/desktop.py`](https://github.com/Tensorera/agenstro/blob/main/segno-flow/src/segno_flow/desktop.py) |
| internal `segnod` composition | `import`, `enable`, `list`, `run`, `status` with explicit `--root` | [`segnod/main.rs`](https://github.com/Tensorera/agenstro/blob/main/segno-flow/rust/segnod/src/main.rs) |

`segnod` is an internal composition binary, not a replacement public command.
There is no public umbrella `agentro` CLI in the current source.
