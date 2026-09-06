# Agenstro

Agenstro helps people work through complex tasks in their existing coding agent.
Initialize a project, open Claude, Codex or another agent there, and ask it to use
the Motivo skill. The agent chooses a focused method, inspects its evidence and
organizes the next action. Haskell scripts express the atomic work units, Clef
composes their calls, and Tactus supervises execution.

Motivo Studio is a self-contained, read-only HTML report. It does not select
models, own a task loop or replace the coding-agent conversation.

The current release line is `0.3`.

## The parts

| Component | Responsibility |
| --- | --- |
| [Clef](clef-sdk/) | Small Haskell workflow, task, effect and plugin abstractions |
| [Tactus](tactus-runtime/) | Script discovery, execution, provider/plugin supervision and diagnostics |
| [Motivo](skills/motivo/SKILL.md) | Methods for clarification, investigation, analysis, research, experiments, organization, retrospection and handoff |
| [Motivo Studio](motivo-studio/) | Offline HTML that makes recorded evidence and decisions readable |
| [Segno](segno-flow/) | Experimental persistent scheduling and business-state checkpoints |
| [motivo.test](plugins/motivo-test/) | One bounded, isolated Linux experiment, limited to its sample directory and at most 600 seconds |

The musical names describe distinct responsibilities. A clef provides a frame,
tactus provides the pulse of execution, motivo is a reusable method, and segno
marks persistent continuation. Plugins remain language-neutral through
`agenstro.plugin/v1`.

## User entry point

```text
User <--> existing coding agent <--- Motivo skill / Tactus skill
                    |
                    +--> .tactus/motivoscript: selected method templates
                    +--> .tactus/scripts: atomic business scripts
                                   |
                              Clef + Tactus
                                   |
                           providers / plugins
                                   |
                         evidence and artifacts
                             |             |
                       main agent     offline HTML --> user
```

Methods are independent choices. They do not force a task through eight stages,
and do not launch another model merely to record something the main agent
already knows. A separate provider call is explicit and uses existing Tactus
configuration; it does not inherit a native interactive session.

## Install from this checkout

Requirements are Rust/Cargo, GHC/Cabal with `base >=4.20 && <4.23`, and the chosen
coding-agent CLI for live work. Linux experiments additionally need usable
Bubblewrap user namespaces. Reading reports does not need Node.js or a server.

```sh
cargo install --path tactus-runtime --bin tactus --locked --force
cargo install --path plugins/motivo-test --locked --force
```

Then follow the three-step path:

1. In your project, run `tactus init --sdk /path/to/Agenstro/clef-sdk`.
2. Start your chosen coding-agent CLI in that folder.
3. Ask the agent to use Motivo to investigate, organize or review the task.

Initialization installs the canonical skills and supported project-local host
pointers. Existing user files are preserved. If your host does not expose skills,
ask it to read `.tactus/skills/motivo/SKILL.md` directly.

Open `.tactus/motivo/index.html` in a browser when you want to inspect progress.
Refresh loads the latest generated snapshot. Keep instructions, model choices
and feedback in the original agent conversation.

See [Installation](docs/install.md) for platform details and existing-workspace
upgrades. The strict `motivo.test` backend is currently Linux only; unsupported
isolation refuses execution rather than falling back to an ordinary shell.

## Workspaces and atomic scripts

```text
.tactus/scripts/       business scripts
.tactus/motivoscript/  method templates and Haskell helpers
.tactus/motivotest/    experiment inputs/outputs by run and sample
.tactus/motivo/        method records, artifacts and HTML reports
.tactus/runs/          Tactus execution diagnostics
```

Tactus defaults to business scripts. An explicit source directory selects Motivo:

```sh
tactus list --scripts-dir .tactus/motivoscript --json
tactus check --scripts-dir .tactus/motivoscript \
  .tactus/motivoscript/020_investigate.hs
```

The main agent fills a method's Markdown input and invokes its stable template.
For actual business changes it writes and runs the appropriate `.tactus/scripts`
entry using the [Tactus skill](skills/tactus/SKILL.md). Default `tactus run --all`
does not include Motivo methods.

Atomic means a coherent, independently inspectable work unit. It does not mean
transactional rollback. Reports distinguish observations from interpretations;
recording a method or receiving exit code zero does not prove task correctness.

## Evidence and experiments

Method records preserve the question, samples, source locations, judgments and
next useful action. Retrospectives compare expectations with observations and
explain changes in direction. Unknown model identity stays unknown; caller
labels and requested provider models are not presented as verified facts.

The probe method calls the `motivo.test` effect. One experiment writes only to
`.tactus/motivotest/<run>/<sample>`, reads the project at `/project`, runs in
`/work`, and shares one deadline across preparation and its command tree. The
600-second maximum cannot be disabled with zero. Timeouts and uncertain results
are retained for inspection, not automatically retried.

These restrictions belong to the experiment plugin. Ordinary trusted Haskell,
project plugins and native agent sessions retain their own authority. Clef and
Tactus do not acquire a universal domain validator or sandbox policy.

## Optional persistent tasks

Segno is unchanged and remains experimental, single-node and at least once.
Install it only when work needs persistent triggers and business-state checkpoints:

```sh
cabal build --builddir=Build/cabal all --enable-tests
cabal install segno-flow:exe:segno \
  --builddir=Build/cabal \
  --installdir "$HOME/.local/bin" \
  --overwrite-policy=always
```

See the [Segno guide](docs/segno.md). Persistence does not provide exactly-once
external effects or automatically restore a workspace.

## Documentation and verification

- [Motivo methods and reports](docs/motivo-studio.md)
- [First business workflow](docs/getting-started.md)
- [Provider setup](docs/providers.md)
- [Clef guide](docs/clef.md)
- [Workspace and configuration](docs/tactus-workspace.md)
- [Plugin authoring](docs/plugin-authoring.md)
- [Observability](docs/observability.md)
- [Operations and recovery](docs/operations.md)
- [Architecture](docs/architecture.md)
- [Support matrix](docs/reference/support-matrix.md)

The canonical model-free repository checks are:

```powershell
./scripts/quality.ps1 -Profile Fast
./scripts/quality.ps1 -Profile Full
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for component checks. Build products stay
under `Build/` or ignored tool directories. The former Electron client and its
installers have been removed; existing `.motivo/tasks` files are historical and
are not silently deleted or resumed.

## Execution boundary

Trusted workflows and configured plugins can perform external work with the
caller's authority. `OutcomeUnknown` means work may have happened without a
trustworthy terminal result. Inspect the actual outcome before deciding to retry.
The dedicated experiment plugin has a narrower, tested Linux boundary; that does
not confer the same restriction on arbitrary code or on the main agent.

Read [SECURITY.md](SECURITY.md) for the project boundary.

## License

Agenstro is licensed under [GNU Affero General Public License v3.0 only](LICENSE),
SPDX `AGPL-3.0-only`. Modified network offerings must meet the corresponding-source
requirements described by that license. Release changes are in [CHANGELOG.md](CHANGELOG.md).
