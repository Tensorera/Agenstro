---
title: Agenstro public roadmap
status: alpha
owners: [release]
last_verified: 2026-08-20
applies_to: "Agenstro 0.3"
platforms: [windows, ubuntu]
---

# Agenstro public roadmap

This page communicates product direction, not an internal issue backlog or a
delivery-date promise. Current guarantees belong in reference pages; planned
work appears here only when it changes what users can reasonably expect.

## Current 0.3 foundation

The release candidate path contains:

- Clef as a compact GHC2021 EDSL for typed provider, effect, and generic plugin
  calls, typed norms, composable rubrics, and bounded refinement;
- Tactus as one Rust CLI/runtime for `.tactus`, process supervision, bounded
  event transport, diagnostic journals, durable decision sessions, and Studio
  control projections;
- Segno as one Haskell, single-node, at-least-once persistent-task driver with
  pure interval/UTC-cron planning and separate SQLite lifecycle/business state;
- Motivo Studio as a thin TypeScript/React/Electron client over Tactus with a
  typed human-decision return channel; and
- a language-neutral one-shot `agenstro.plugin/v1` process boundary.

The release gate covers Windows and Ubuntu source builds, local fake providers,
cross-language Haskell/Rust execution, the multi-step topology example, Segno
virtual-time behavior, Motivo package creation, and strict documentation.

## Release hardening

The immediate direction is to make the existing boundary easier to install,
operate, and extend without enlarging the core:

1. produce reproducible, signed installation artifacts where practical;
2. publish exact provider CLI compatibility and authenticated optional smoke
   evidence without putting credentials in CI;
3. improve bounded cancellation and process-tree acceptance across platforms;
4. add plugin author kits and shared conformance vectors for Rust, TypeScript,
   C#, and Haskell;
5. formalize run-retention and diagnostic export commands;
6. add supported reconciliation operations for Segno `OutcomeUnknown`; and
7. stabilize the public control DTOs used by Motivo.

## Clef direction

Clef should remain ordinary typed Haskell rather than grow a custom parser,
hidden DAG, or closed effect universe. Compatible additions include better
typed wrappers, reusable task libraries, documented error recovery, and
observation APIs that remain orthogonal to workflow values. Norm catalogue
policy, SARIF export, and reviewable mining are staged follow-ups; automatic
promotion of mined norms is not implied.

## Tactus direction

Tactus should remain a small, dependable execution kernel rather than become a
network daemon. Compatible work includes packaging, config migration tooling,
bounded run inspection/retention, provider adapter acceptance, and stronger
cross-platform descendant cleanup. Session planner registration and
`session advance` require an explicit invocation contract before they become
runtime commands.

## Segno direction

Segno's next useful work is operational rather than distributed: explicit
reconciliation, configurable policies, lease renewal for very long tasks,
additional trigger/state plugins, and better backup/inspection. Multi-node
coordination would require a new consistency design and is not implied by the
current SQLite driver.

## Motivo direction

Motivo should improve visibility and safe named actions while remaining a
projection. Candidate work includes richer trace filtering, accessibility,
signed installers, transcript projection, and typed configuration mutation
through future Tactus commands. Direct TOML/database/session parsing, a general
terminal, and private runtime ownership remain out of scope.

## Explicit non-goals for 0.3

The current line does not promise:

- hostile-code sandboxing;
- a hosted or multi-user Tactus service;
- credential brokerage or authentication;
- exactly-once external effects;
- automatic rollback of files, Git, providers, or plugins;
- serialization of arbitrary Haskell continuations;
- deterministic replay of arbitrary Haskell `IO`;
- distributed Segno consensus; or
- automatic migration of removed 0.2 daemon/scheduler state.

Scheduling and replay are different. Segno creates a new occurrence and asks
Tactus to execute its Clef task; Tactus diagnostics are never substituted for
that computation.

## How a roadmap item becomes a guarantee

A feature is current only after all of the following are true:

1. the authoritative implementation exists;
2. boundary and failure tests exercise it;
3. the relevant reference page defines it;
4. the support matrix lists its platforms and limits; and
5. the change appears in the changelog for a concrete commit or release.

Architecture ideas and ADR discussion alone are not release guarantees.
