# Agenstro 0.3 documentation

Agenstro starts in the user's existing coding agent: initialize the project,
open the chosen agent there, and ask it to use Motivo. The Motivo skill offers
eight Haskell methods and offline HTML reports. Tactus supervises execution,
Clef supplies typed Haskell composition, and Segno owns persistent typed
triggers/state.

## Start here

| Goal | Canonical page |
| --- | --- |
| Install or upgrade the commands | [Installation](install.md) |
| Build one model-free workflow | [First workflow](getting-started.md) |
| Select a provider, model, or effort | [Provider setup](providers.md) |
| Investigate, organize, or review a task from the current agent | [Motivo methods and reports](motivo-studio.md) |

## Develop workflows and capabilities

| Goal | Canonical page |
| --- | --- |
| Learn `Workflow`, tasks, effects, plugins, and errors | [Clef workflow guide](clef.md) |
| Understand `.tactus`, TOML, and script ordering | [Tactus workspace and configuration](tactus-workspace.md) |
| Implement a one-shot capability | [Plugin authoring](plugin-authoring.md) |
| Build a persistent typed task | [Segno persistent tasks](segno.md) |
| Look up exact frames | [Local plugin protocol v1](reference/plugin-protocol-v1.md) |

## Operate and diagnose

| Goal | Canonical page |
| --- | --- |
| Understand state, human messages, and journals | [Logs and run evidence](observability.md) |
| Back up, restore, retain, or upgrade a workspace | [Workspace operations](operations.md) |
| Diagnose a symptom | [Troubleshooting](troubleshooting.md) |
| Check supported platforms and guarantees | [Support matrix](reference/support-matrix.md) |
| Resolve terminology | [Glossary](reference/glossary.md) |

## Component responsibility

| Component | Responsibility | Explicit non-responsibility |
| --- | --- | --- |
| Clef | Typed Haskell workflow and persistent-task values | Provider catalogue, scheduling loop, sandbox |
| Tactus | Workspace, process supervision, protocol routing, diagnostic evidence | Workflow semantics, credentials, rollback, replay |
| Segno | Single-node triggers, occurrences, leases, fences, SQLite state | Exactly-once effects, distributed consensus, provider execution |
| Motivo | Project skill, eight method templates, `.tactus/motivo` evidence, offline HTML snapshots | Main conversation, provider registry, process kernel, autonomous task loop, scheduler |

The names follow a musical coordination metaphor: Clef establishes the typed
frame, Tactus supplies the execution pulse, Segno marks persistent continuation,
and Motivo supplies reusable methods of task investigation and reflection.
Method scripts live in `.tactus/motivoscript`, separate from business scripts.
Minimal experiments use `.tactus/motivotest` through the `motivo.test` plugin.

## Safety in one paragraph

Workflow code, plugins, and coding-agent CLIs run with the current user's
operating-system authority. `generate` and live provider calls may contact or
bill external services. Tactus is not a sandbox, credential broker, backup,
or rollback engine. `OutcomeUnknown` means an external effect may have happened
without a trustworthy terminal result and must be reconciled before retry.
The Linux `motivo.test` backend enforces a separate experiment write boundary
and a deadline of at most 600 seconds; unsupported backends refuse experiments.
A completed method report does not certify that the user's task is solved.

## Project and contributor material

- [Architecture](architecture.md) explains ownership and data flow.
- [CLI reference](reference/cli-v0.3.md) lists supported commands.
- [Segno plugin wire](reference/segno-plugin-wire-v1.md) defines trigger/state backends.
- [Studio control API](reference/studio-control-v1.md) defines Tactus workspace projections.
- [Agent-led Motivo methods](adr/0008-agent-led-motivo.md) keeps the conversation
  in the original coding agent and replaces the desktop task loop with methods
  and offline observation, without changing Segno persistence.
- [Public roadmap](roadmap.md) separates current guarantees from later work.
- [ADR-0003](adr/0003-haskell-dsl-and-local-plugins.md) and
  [ADR-0004](adr/0004-haskell-segno-persistent-tasks.md) retain design rationale.
- [Migration 0.2 to 0.3](migrations/0.2-to-haskell-0.3.md) is historical upgrade
  context, not the new-user path.

## License

Agenstro source is licensed under GNU AGPL v3.0 only (`AGPL-3.0-only`). See the
repository `LICENSE` file for the complete terms.
