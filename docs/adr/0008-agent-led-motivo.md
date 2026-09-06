---
title: Agent-led Motivo methods and offline observation
status: working decision record
owners: [architecture, motivo]
last_verified: 2026-09-05
applies_to: "Agenstro 0.3"
platforms: [windows, ubuntu]
---

# ADR-0008: The existing coding agent owns the conversation

- Status: Accepted
- Supersedes: ADR-0007's desktop-owned task method and execution loop
- Preserves: Clef composition, Tactus process/protocol ownership, Segno

## Decision

The user initializes a project, starts their chosen coding agent there, and asks
that agent to use the Motivo skill. Motivo provides independently selectable
Haskell method templates and evidence reports. It does not create another agent
client or autonomously schedule a sequence of methods.

Templates live in `.tactus/motivoscript`, separate from business entries in
`.tactus/scripts`. Tactus accepts a generic `--scripts-dir` on list/check/run;
default selection still uses only business scripts. Clef gains no method types.

The eight methods are clarify, investigate, analyze, research, probe, organize,
retrospect, and handoff. They are choices, not required stages. The main agent
can submit existing Markdown, or explicitly select an existing Tactus provider
when independent context is useful. No implicit provider switch or native
conversation inheritance is promised. Requested and observed model identity
are distinct; unknown identity stays unknown.

A `motivo.test` effect implements bounded local experiments. On its Linux
backend, a Bubblewrap filesystem/PID namespace confines persistent writes to
`.tactus/motivotest/<run>/<sample>`. One monotonic experiment deadline covers
preparation and execution; the default and maximum are 600 seconds. An unavailable
isolation backend refuses execution. This is a specific plugin contract, not a
new universal security policy in Clef or Tactus.

Method records and artifacts live in `.tactus/motivo`. They reference Tactus run
IDs but do not replace or replay runtime journals. A report's completion says
nothing automatic about completion of the user's engineering goal.

Motivo Studio is a self-contained read-only HTML projection. It accepts no task,
provider, or execution request. Core content is pre-rendered with inline styles;
small reading controls need neither a port nor adjacent-file fetch. Refresh
loads a newer snapshot. Recorded timestamps describe evidence, not a live
connection to an agent.

## Migration

Remove the Electron main/preload/IPC, task service, provider transport, execution
views and installers. Retain useful presentation ideas and stable Tactus APIs.
Existing `.motivo/tasks` files remain historical and are not automatically
deleted, converted into new evidence, or resumed.

## Consequences

Method guidance is owned by the skill and editable Haskell templates. Users
retain their existing model/permission interface and can inspect the resulting
evidence without another client. Method overhead and actual outcomes still
need evaluation against direct-agent work; template compliance is not a proof
of correctness or improved model capability.
