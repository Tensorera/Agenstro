---
title: Agenstro roadmap
status: alpha
last_verified: 2026-08-15
applies_to: "Clef Haskell 0.3 + Tactus Rust 0.3 source-alpha direction"
---

# Agenstro roadmap

This page records architectural direction and acceptance gates, not promised
dates. A feature is current only when implementation, tests, documentation, and
the stated platform gate agree.

## Status vocabulary

| Label | Meaning |
| --- | --- |
| Current gate | Required behavior for the Clef/Tactus/Motivo `0.3` path |
| Hardening candidate | Fits the architecture but is not yet a stable promise |
| Exploratory | Needs a bounded design/prototype before support can be claimed |
| Frozen | Retained source with no current feature or packaging claim |
| Non-goal | Intentionally absent from the core |

## Current `0.3` source gate

The current pair is Clef `0.3.0.0` and Tactus `0.3.0`:

- Clef is a small GHC2021 EDSL with typed provider, effect, and generic plugin
  calls;
- plugin events are parsed incrementally and projected through an `EventSink`
  outside the workflow result type;
- Tactus is a typed Rust CLI/runtime exposing `init`, `list`, `prompt`,
  `generate`, `check`, `run`, `doctor`, `smoke`, and `plugin-call`;
- Tactus owns one-shot process-group supervision, bounded incremental frame
  routing, deadlines/cancellation, and `agenstro.trace/v1` run journals;
- providers, effects, and general plugins share the language-neutral
  `agenstro.plugin/v1` JSONL boundary;
- Rust adapters cover Codex, Claude Code, OpenCode, and `workspace.paths`;
- fake-driven tests make no authenticated model request; and
- Motivo Studio is a typed Electron/React projection over the versioned Tactus
  Studio control API; Segno Flow remains frozen outside the gate.

The source gate includes Haskell build/tests, Rust format/check/test/clippy,
protocol/adaptor tests with local fakes, an offline Tactus-to-GHC path, the
multi-step topology example, Motivo format/lint/typecheck/Vitest/package, and a
strict documentation build. Rust verification must use a temporary
`CARGO_TARGET_DIR` and clean it afterward so the repository does not accumulate
gigabytes of build output.

A green source gate does not imply a signed installer, stable public API, live
provider certification, hostile-code sandbox, or production-service readiness.

## Immediate alpha acceptance work

1. Keep package versions, install commands, runtime configuration, examples,
   support matrix, and actual CLI help synchronized.
2. Preserve Windows and Linux tests for process-group cleanup, Unicode JSONL,
   incremental event timing, and strict config/protocol failures.
3. Keep a first-run offline path reproducible before any provider credentials
   or billing are involved.
4. Make every live entry (`generate`, provider `plugin-call`, and `smoke
   --live`) visibly opt-in in docs and output.
5. Keep `.tactus/runs`, credentials, local config, provider transcripts, and
   private prompts out of source control.
6. Publish evidence for the exact commit/tag before describing a platform or
   native provider version as verified.
7. Preserve the trust statement: process groups and argv execution improve
   runtime correctness, not authorization or isolation.

## Hardening candidates

### Stabilize trace retention and redaction

`agenstro.trace/v1` currently records factual ordered events and an atomic
terminal summary. Motivo consumes only a bounded Rust projection, not the disk
layout. Before any tool treats traces as a durable/replay API, define:

- retention and explicit cleanup ownership;
- redaction hooks for prompts, provider events, and diagnostics;
- schema compatibility and migration rules;
- limits for summary/event payloads and total local storage; and
- the distinction between plugin frames, controller evidence, compiler output,
  and arbitrary workflow `IO`.

This must not retrofit the current journal into a claim of deterministic
replay.

### Broaden process-supervision evidence

The Rust kernel already owns Unix process groups/Windows Job Objects, bounded
pipes, cancellation, and deadlines. Additional evidence should cover:

- descendants that inherit pipes or spawn immediately before cancellation;
- slow event sinks and queue backpressure;
- very large Unicode input written concurrently with stdout/stderr;
- repeated Ctrl+C and cleanup races; and
- disk-full/permission failure while appending events or publishing a summary.

Failure reporting must remain factual. Killing a local process cannot prove a
remote provider did not complete.

### Expand plugin author ergonomics

The wire contract intentionally accepts any language. Candidate additions are:

- tiny conformance fixtures for Rust, Haskell, TypeScript, and C#;
- a command that feeds malformed/edge-case frames to a plugin test harness;
- generated JSON Schema for requests, frames, runtime config, and trace
  envelopes where it improves tooling; and
- typed Haskell convenience modules for stable popular plugins.

None should create a mandatory plugin language, manifest-signing system, or
closed method/event enumeration.

### Expand provider compatibility evidence

Default CI should remain fake-driven. Optional credentialed acceptance may
record native CLI version, OS, selected model, and observed capability, without
becoming a required secret-bearing job.

Compatibility claims must keep permission semantics distinct:

- Codex and Claude Code expose explicit dangerous bypass flags;
- OpenCode `--auto` plus `permission=allow` cannot override every explicit deny
  or managed policy; and
- model, effort, and variant values are open and provider-specific.

Adapter flags, parsers, tests, support matrix, and troubleshooting guidance must
change together when a native CLI evolves.

### Packaging and developer ergonomics

Candidates include:

- reproducible source archives and checksums;
- a documented install/upgrade/uninstall story beyond `cargo install --path`;
- generated Haddock and Rust API documentation;
- pinned documentation dependencies and link checks;
- automatic detection/reporting of a large repository-local `target/`; and
- a packaged offline example derived from `examples/topology-holes`.

Signed installers, auto-update, and background services are not implied.

### Motivo Studio hardening

Motivo's initial current role is intentionally complete but narrow: workspace
health, scripts, registries, actions, and bounded trace pages. Follow-up work may
add signed installers, stronger cross-platform cancellation evidence, persistent
non-secret UI preferences, and richer trace filtering. It must not become a
second runtime, daemon, general shell, credential store, or approval-policy
owner.

## Exploratory surface

### Segno Flow and replay

Segno remains frozen because scheduling and replay require distinct semantics:

1. **Recorded-result replay** substitutes previously captured terminal values
   and must define trace integrity, missing evidence, schema drift, and secret
   handling.
2. **Live re-invocation** intentionally repeats provider/effect calls and can
   incur new cost and external-state changes.
3. **Ordinary Haskell `IO`** is not intercepted by Clef and is generally not
   replayable.

Scheduling, retry, and replay are not synonyms. No Segno revival should begin
until trace ownership and substitution rules are explicit.

## Current non-goals

The `0.3` core does not promise:

- hostile-code isolation for Haskell or plugins;
- plugin signing, authentication, capability tokens, approval UI, or a
  credential broker;
- a daemon, socket API, service discovery, or persistent provider session;
- a global static DAG for arbitrary Haskell control flow;
- artifact publication, CAS, checkpoint restore, workspace transactions, or
  Git rollback;
- exactly-once provider calls/effect cleanup or automatic retry;
- read/transient-write detection or attribution by `workspace.paths`;
- permission equivalence among Codex, Claude Code, and OpenCode;
- automatic translation of historical cell/notebook workflows or scheduler
  state;
- authenticated provider calls in ordinary CI; or
- deterministic replay of arbitrary `IO`.

Moving an item out of this list requires a scoped design, tests at the claimed
boundary, migration impact, and honest residual limitations.

## How a roadmap item becomes supported

A proposal becomes current only when all applicable evidence exists:

1. an accepted, bounded contract;
2. code in the authoritative Haskell Clef or Rust Tactus path;
3. deterministic tests, including platform-specific cases where relevant;
4. a support-matrix entry separating fake/offline/live evidence;
5. user documentation and failure guidance;
6. migration and removal consequences; and
7. a green gate for the exact commit/tag.

See [Architecture](architecture.md), [Getting started](getting-started.md), and
the [support matrix](reference/support-matrix.md) for current behavior.
