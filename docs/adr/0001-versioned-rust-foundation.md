# ADR-0001: Versioned Rust Foundation

- Status: Accepted
- Date: 2026-08-01
- Scope: M0/M1 foundation only

This ADR records the foundation decision at M0/M1. The current `0.2.0` source
has since added product composition and cross-language DTO tests. See
[ADR-0002](0002-daemon-state-and-electron-boundaries.md) for the current daemon,
database, external CAS, Python worker, and Electron ownership decision.

## Context

The greenfield implementation needs one control-plane foundation without
turning the prototype Python runtimes into dependencies. The existing public
projects and distributions remain `clef-sdk`, `tactus-runtime`,
`motivo-studio`, and `segno-flow`. Their public Python imports and executable
entry points remain unchanged.

The target architecture requires Rust domain types to remain independent of
Protobuf-generated transport types. It also requires one versioned IDL source,
bounded list and capability contracts, stable machine errors, and a canonical
digest that does not depend on ordinary Protobuf serialization.

## Decision

1. `PreviewVer/Cargo.toml` is the Rust 2024 workspace root and uses resolver 3.
   The initial MSRV is Rust 1.88 because the selected published Tonic 0.14 line
   requires it. Applications commit `Cargo.lock`; CI checks MSRV and current
   stable independently.
2. `proto/` is the only API v1 IDL source. Packages use
   `agentro.<bounded_context>.v1`, proto3 presence rules, and Buf STANDARD lint.
   Initial messages cover common error, capability, digest, pagination, health,
   and server-info contracts.
3. `agentro-contracts` owns pure domain values. It may depend on pure value and
   hashing libraries, but not Tokio, SQLite, filesystems, Tonic, Prost, OS APIs,
   or provider SDKs.
4. `agentro-proto` owns generated Prost/Tonic code and the descriptor set. Its
   build script uses the Cargo-locked `protoc-bin-vendored` executable directly;
   it does not mutate process-global environment variables and generated types
   never enter domain crates.
5. Canonical SHA-256 input uses the explicit `agentro-canonical-v1` tagged,
   length-prefixed encoding with sorted field names and hard field limits.
   Protobuf bytes are not canonical digest input.
6. Product core members live inside `clef-sdk/rust/clef-core`,
   `tactus-runtime/rust/tactus-core`, and `segno-flow/rust/segno-core`. At this
   milestone they establish identity and dependency direction only; they do not
   claim to implement the three products.
7. The checked-in boundary script uses `cargo metadata` to reject transport,
   async-runtime, and storage dependencies in domain crates. It also verifies
   the four public distribution names, three Python import names, and existing
   CLI/GUI entry points. Each product core must directly depend on
   `agentro-contracts`, but may add other pure domain dependencies whose full
   normal dependency closure passes the same forbidden-dependency gate.

## Consequences

- Daemon handlers will explicitly validate and convert generated wire values to
  domain values. This costs small conversion code but prevents wire evolution
  from becoming domain state.
- `agentrod`, `tactusd`, and `segnod` remain separate future process owners.
  This ADR does not create a combined daemon or shared database.
- The first released descriptor snapshot will become the baseline for
  `buf breaking`. There is no prior greenfield release against which the
  initial schema can run a meaningful breaking check.
- Python SDK generation, TypeScript generation, daemon authentication,
  persistence, worker execution, process containment, and cross-language
  interoperability remain later milestones and must not be inferred from this
  foundation compiling.

## Verification

The local foundation gate is:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked --document-private-items
python -m unittest discover -s scripts/tests -v
python scripts/check_workspace_boundaries.py
buf format --diff --exit-code
buf lint
```

The CI skeleton runs Rust on Windows and Ubuntu, adds an Ubuntu MSRV job, pins
GitHub actions by commit, and installs Buf 1.72.0 explicitly.

## Current Implementation Note

As of `0.2.0`, Tactus and Segno have independently tested SQLite composition,
Tactus has an external CAS checkpoint backend, and Motivo has an Electron
main/preload/renderer boundary. `agentrod` persistence and real authenticated
daemon listeners remain absent. Compilation of the foundation must not be used
as evidence for those missing release capabilities.
