---
name: segno
description: Write and operate Segno persistent Tactus tasks. Use for recurring local workflows, standard .tactus/segno.hs organizers, timestamp-based work directories, machine-local schedule registration, or paused/unknown invocation recovery with segno service. Existing typed Segno install/driver workflows use a separate interface; inspect their installed CLI and sources before changing them.
---

# Segno persistent tasks

Use the installed `segno` and `tactus` commands. Segno's machine-local service
registers one `.tactus/segno.hs` organizer per workspace and runs it on an
interval. Tactus executes the Haskell organizer and its business scripts.

- For writing or changing an organizer, read
  [references/organizer.md](references/organizer.md). The bundled
  [organizer](assets/segno.hs) and [business script](assets/010_business.hs)
  form a model-free starting point. Adapt them to the requested task; preserve
  existing workspace code and state.
- For registering, starting, observing, pausing, or recovering scheduled work,
  read [references/service.md](references/service.md). Inspect the selected
  registry and any existing service process before starting another scheduler.

Keep three responsibilities separate: the central registry owns schedules
and invocation history; the organizer owns occurrence directories and business
completion state; numbered business scripts accept an explicit work-directory
argument. Use task-specific evidence to decide whether an external effect
already completed. A timestamp directory is not an idempotency guarantee.

Type-check selected sources before running them. Registering makes the first
invocation immediately due, so an existing service can execute it at once.
Carry out registration, execution, and recovery when the user's task already
authorizes them; a request to write a script alone does not authorize enabling
a recurring job or startup service.

Return a known nonzero business status only from an explicit, understood
business outcome. A timeout, failed subprocess, missing response, or ambiguous
external effect is uncertainty: let Segno record `outcome_unknown` and pause.
Inspect artifacts, invocation history, and external effects before recovery;
resuming schedules a new future timestamp and does not replay the old one.
