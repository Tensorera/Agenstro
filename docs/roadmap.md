---
title: Agenstro roadmap
status: alpha
last_verified: 2026-08-15
applies_to: "0.3 source-alpha direction"
---

# Agenstro roadmap

This roadmap records direction and acceptance gates, not promised dates. Items
outside the current gate remain proposals until code, tests, documentation, and
an explicit design decision exist. Provider products and their command-line
interfaces also evolve independently of this repository.

## Status vocabulary

| Label | Meaning |
| --- | --- |
| Current gate | Required for the Clef/Tactus 0.3 source path and expected to remain green |
| Hardening candidate | A concrete next improvement, not yet a release promise |
| Exploratory | Needs an ADR or prototype before it can define support |
| Frozen | Retained as evidence; no active feature work or current support claim |
| Non-goal | Intentionally absent from the current architecture |

## Current 0.3 source gate

The current release candidate is the combination of Clef `0.3.0.0` and Tactus
`0.3.0`:

- Clef is an ordinary Haskell EDSL using GHC2021 and
  `base >=4.20 && <4.23`;
- Tactus initializes a project-local workspace and provides `init`, `list`,
  `prompt`, `generate`, `check`, `run`, `doctor`, and `smoke`;
- providers and effects use the one-shot `agenstro.plugin/v1` JSONL boundary;
- reference adapters exist for Codex, Claude Code, and OpenCode;
- `workspace.paths` provides observational path-difference evidence;
- Windows and Ubuntu are configured as required Haskell/Tactus source jobs;
- CI provider coverage uses fakes and makes no authenticated model request; and
- Motivo Studio, Segno Flow, and the previous Rust/Python runtime paths are
  excluded from the gate.

The source gate is complete only when build, unit, protocol, static-analysis,
and end-to-end Tactus-to-GHC jobs pass on the declared matrix. A successful
source gate does not imply a signed installer, stable public API, live provider
certification, security sandbox, or production-service readiness.

## Release-readiness work for the 0.3 alpha

These are the immediate acceptance concerns for the private source release:

1. Keep Clef and Tactus versions, package metadata, examples, support matrix,
   and migration language consistent.
2. Run the complete Windows/Ubuntu CI workflow from the published repository
   and preserve a linkable result before describing the matrix as verified.
3. Ensure the first-run path is reproducible without a model call, then keep
   live smoke and generation visibly opt-in.
4. Keep provider credentials and local configuration out of Git; publish only
   fake transcripts and redacted evidence.
5. Distinguish current 0.3 documentation from browsable 0.2 legacy pages so a
   GitHub reader cannot mistake old daemon, Python-cell, Motivo, or Segno
   instructions for supported behavior.
6. Keep the Windows legacy-codepage subprocess regression green so reference
   plugin hosts continue to force UTF-8 independently of the active code page.
7. Preserve the explicit trust statement: argument arrays avoid shell parsing,
   but plugins and Haskell programs are still arbitrary local code.

No compatibility guarantee should be attached to an alpha tag until these
claims match the exact tagged source and CI evidence.

## Hardening candidates

The following work fits the current architecture but is not promised by a date
or version.

### Extend UTF-8 interoperability coverage

The reference Python provider/effect hosts now configure their real standard
streams as UTF-8 and have a subprocess regression with `PYTHONUTF8=0` and a
CP936 standard-stream setting. Further hardening should exercise external
plugins in other languages and keep a documented environment-variable fallback
for older editable installations. The wire encoding itself remains UTF-8.

### Bound resource use and clarify cancellation

The one-shot protocol currently has no core default deadline or output quota,
and clients buffer protocol output until exit. Candidates include:

- explicit per-plugin deadlines with documented defaults and overrides;
- bounded stdout, stderr, event count, line length, and terminal value size;
- process-tree cancellation behavior on Windows and Ubuntu;
- a precise `outcome_unknown` and retry-safety contract; and
- tests for descendants retaining pipes or ignoring termination.

Limits must fail honestly. A timeout cannot be documented as rollback or proof
that an external provider did not complete.

### Stabilize trace and evidence records

Clef retains provider events and effect evidence, but the first protocol is not
a replay format. Before another component consumes traces, define:

- versioned record envelopes and ordering;
- redaction and credential-handling rules;
- bounded retention and storage ownership;
- the distinction between provider output, effect evidence, diagnostics, and
  workflow return values; and
- cancellation and partial-observer records.

This work is a prerequisite for any credible Segno replay design.

### Expand adapter compatibility evidence

Fake-driven CI should remain deterministic. Optional, explicitly credentialed
acceptance can record native CLI version, operating system, selected model, and
observed capabilities without becoming a required public-CI secret path.

Compatibility evidence must not claim equivalent permission semantics:

- Codex and Claude Code expose explicit high-authority non-interactive flags;
- OpenCode `--auto` plus inline `permission=allow` cannot override every deny or
  managed configuration; and
- provider model/effort/variant values remain open and provider-specific.

When a native CLI changes its output schema or flags, the adapter, tests,
support matrix, and troubleshooting page should change together.

### Improve package and developer ergonomics

Candidates include:

- generated Haddock/API documentation for the tagged Clef package;
- a small offline example installed or packaged with each source release;
- reproducible documentation dependencies and link checking;
- clearer source distribution checks and checksums; and
- an explicit install, upgrade, uninstall, and state-removal story if the
  project moves beyond editable source installs.

Signed installers, automatic update, background services, and stable package
registry publication are not implied by these candidates.

### Grow typed plugin convenience modules selectively

The open JSON protocol should remain language-neutral. Frequently used effects
may add typed Haskell wrappers like `Clef.Effect.WorkspacePaths` without turning
the core into a global type-level effect row. A wrapper is justified when it
improves result wiring and documents a stable plugin contract; it must retain an
escape hatch for plugin-specific JSON options.

## Exploratory work after the core boundary stabilizes

### Motivo Studio

Motivo is frozen during the 0.3 cutover. If revived, the smallest coherent role
is a projection of Tactus workspace, plugin discovery/description, smoke
results, and bounded runtime evidence. It must not silently reintroduce a
second workflow runtime, daemon owner, credential store, or approval policy.

Revival requires at least:

- a stable Tactus inspection surface;
- an explicit Electron security review;
- a packaging and native-dependency plan for each supported platform;
- UI language that distinguishes offline, live, and outcome-unknown states; and
- an ADR assigning process, credential, and failure ownership.

Until those gates exist, the checked-in Electron 0.2 code remains frozen
evidence rather than a 0.3 product claim.

### Segno Flow and replay

Segno's existing cron/lease scheduler is not a replay engine. Any new proposal
must distinguish at least:

1. **Recorded-result replay**, which reuses captured provider/effect results and
   must define trace integrity, missing evidence, and deterministic substitution;
2. **Live re-invocation**, which intentionally contacts providers or repeats
   effects and therefore carries new cost, authorization, and external-state
   risk; and
3. **Ordinary Haskell `IO`**, which is generally not interceptable or replayable
   by Clef.

Scheduling, retry, and replay must not be treated as synonyms. Segno remains
frozen until trace semantics and ownership are explicit.

## Current non-goals

The 0.3 architecture does not promise:

- hostile-code isolation for Haskell workflows or plugins;
- plugin signing, authentication, capability tokens, or a credential broker;
- a daemon network, service discovery, or persistent provider sessions;
- global static DAG compilation for arbitrary Haskell control flow;
- artifact publication, CAS, checkpoint restore, or Git rollback;
- exactly-once provider calls or effect cleanup;
- reads or transient-write detection by `workspace.paths`;
- automatic translation of 0.2 Python workflows, cells, state, or schedules;
- permission equivalence across Codex, Claude Code, and OpenCode;
- a live provider call as part of ordinary CI; or
- replay of arbitrary `IO`.

Moving any item out of this list requires a scoped design, tests at the claimed
boundary, migration impact, and documentation that states what still remains
unproven.

## How a roadmap item becomes supported

A roadmap item becomes a current claim only when all applicable evidence exists:

1. an accepted design or clearly bounded implementation contract;
2. code in the authoritative 0.3 path rather than only a legacy tree;
3. deterministic tests, including platform-specific cases where relevant;
4. a support-matrix entry that separates fake/offline/live evidence;
5. user documentation and failure guidance;
6. migration and rollback consequences; and
7. a green release gate for the exact commit or tag.

Until then, the conservative interpretation wins. See
[Architecture](architecture.md), [Getting started](getting-started.md), and the
[support matrix](reference/support-matrix.md) for current behavior.
