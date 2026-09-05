# Agenstro 0.3 documentation

Agenstro supports concrete coding-agent tasks and reusable typed workflows.
Motivo Studio owns the task method and local report history; Tactus supervises
execution in Rust. Clef provides Haskell composition when useful, and Segno
adds persistent typed triggers/state.

## Start here

| Goal | Canonical page |
| --- | --- |
| Install or upgrade the commands | [Installation](install.md) |
| Build one model-free workflow | [First workflow](getting-started.md) |
| Select a provider, model, or effort | [Provider setup](providers.md) |
| Work on a goal with bounded agent calls | [Motivo Studio Tasks](motivo-studio.md) |

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
| Motivo Studio | Replaceable task method, `.motivo` task reports, and workspace interface | Tactus config/session/journal ownership, process kernel, scheduler, general shell |

The names follow a musical coordination metaphor: Clef establishes the typed
frame, Tactus supplies the execution pulse, Segno marks persistent continuation,
and Motivo supplies a reusable pattern of task work. Agenstro names the agent
orchestration as a whole.

## Safety in one paragraph

Workflow code, plugins, and coding-agent CLIs run with the current user's
operating-system authority. `generate` and live provider calls may contact or
bill external services. Tactus is not a sandbox, credential broker, backup,
or rollback engine. `OutcomeUnknown` means an external effect may have happened
without a trustworthy terminal result and must be reconciled before retry.

## Project and contributor material

- [Architecture](architecture.md) explains ownership and data flow.
- [CLI reference](reference/cli-v0.3.md) lists supported commands.
- [Segno plugin wire](reference/segno-plugin-wire-v1.md) defines trigger/state backends.
- [Studio control API](reference/studio-control-v1.md) defines Tactus workspace projections.
- [Motivo task method](adr/0007-motivo-task-method.md) defines task ownership
  without changing the Tactus control API or Segno persistence.
- [Public roadmap](roadmap.md) separates current guarantees from later work.
- [ADR-0003](adr/0003-haskell-dsl-and-local-plugins.md) and
  [ADR-0004](adr/0004-haskell-segno-persistent-tasks.md) retain design rationale.
- [Migration 0.2 to 0.3](migrations/0.2-to-haskell-0.3.md) is historical upgrade
  context, not the new-user path.

## License

Agenstro source is licensed under GNU AGPL v3.0 only (`AGPL-3.0-only`). See the
repository `LICENSE` file for the complete terms.
