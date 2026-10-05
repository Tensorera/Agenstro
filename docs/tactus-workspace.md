---
title: Tactus workspace and configuration
status: alpha
owners: [tactus]
last_verified: 2026-09-18
applies_to: "tactus-runtime 0.3.0"
platforms: [windows, ubuntu]
---

# Tactus workspace and configuration

Tactus owns the project-local `.tactus` convention and the typed
`tactus.toml` registry. This page is the canonical reference for workspace
discovery, file ownership, script selection, and configuration fields.

## Discovery

Most Tactus commands accept `--root PATH`. Tactus canonicalizes that path and
searches it and its ancestors for `.tactus/tactus.toml`. The selected directory
containing `.tactus` becomes the workspace root.

```powershell
tactus doctor --root D:\work\project\src
```

The command above can select `D:\work\project` when that ancestor contains the
marker. It does not initialize a second nested workspace.

## Initialization

```powershell
tactus init D:\work\project --sdk D:\src\Agenstro\clef-sdk
```

Initialization creates missing directories and files with create-new
semantics. Existing project-owned content is preserved, so rerunning `init`
does not reset configuration, prompts, scripts, or the Cabal link.

SDK discovery considers, in order:

1. the explicit `--sdk` path;
2. `TACTUS_CLEF_SDK`;
3. `clef-sdk` below the workspace;
4. `clef-sdk` beside the workspace; and
5. the source checkout used to build Tactus.

The selected directory must contain `clef-sdk.cabal`. Prefer an explicit path
for reproducible setup and rerun `doctor` after moving either checkout.

## Directory ownership

```text
.tactus/
  tactus.toml
  cabal.project
  PROMPT.md
  scripts/
  motivoscript/
  motivo/
    index.html
    runs/
  motivotest/
  runs/
  sessions/
  skills/
    tactus/
      SKILL.md
      references/
    motivo/
    segno/
  skills-manifest.json
  skill-updates/
  path-effect/
  dist-newstyle/
  segno.hs
  segno/
```

| Path | Owner and lifecycle |
| --- | --- |
| `tactus.toml` | Project-owned typed registry; edit deliberately and version it when appropriate |
| `cabal.project` | Init-generated link to Clef; repair it after moving the SDK |
| `PROMPT.md` | Project-owned instructions included in workflow-generation requests |
| `scripts/` | Project-owned Haskell entries and helper modules |
| `motivoscript/` | Init-installed Motivo method templates and helpers; existing project edits are preserved |
| `motivo/` | Method inputs, evidence, and generated HTML reports; may contain business material |
| `motivotest/` | Project experiment fixtures and results, organized by run and sample |
| `skills/{tactus,motivo,segno}/` | Installed agent instructions, references and assets; preserve local customizations |
| `skills-manifest.json` | Runtime-owned hashes used to recognize unchanged installed skill files |
| `skill-updates/` | Incoming bundled skill versions preserved for comparison with local edits |
| `runs/` | Runtime-owned bounded diagnostic journals; not replay state |
| `sessions/` | Runtime-owned current elicitation state and append-only decision evidence |
| `path-effect/` | Runtime-owned snapshot/observation tokens |
| `dist-newstyle/` | Rebuildable Cabal output |
| `segno.hs` | Project-owned, explicit-only timestamp organizer for the machine-local service; supplied separately from Tactus initialization |
| `segno/` | Segno-owned job, trigger, lifecycle, and business-state data |

The optional `runtime_instructions` file is also project-owned; its location
comes from configuration and need not be below `.tactus`.

Do not edit runtime-owned state while a command is active. Back up project-owned
files, Motivo and session evidence, and Segno databases before deleting `.tactus`; see
[Workspace operations](operations.md).

`sessions/` is additive for existing workspaces: `session list` returns an
empty list when the directory has not been created yet, while a later
idempotent `tactus init` creates it. Session clients use
`tactus session list/show/answer`; they do not read or write this directory.
Current Motivo methods use the coding-agent conversation and offline reports,
without calling the session API.
See the [session control API](reference/session-control-v1.md).

The three skills have separate responsibilities: Tactus covers workflow
authoring, execution, daemon supervision and live output; Segno covers
timestamp organizers, persistent scheduling and recovery; Motivo covers task
methods and routes execution or scheduling work to the relevant sibling skill.
`tactus init` installs all three and host discovery pointers while preserving
existing files. Use `tactus skills update --dry-run` to inspect changes and
`tactus skills update` to update known unchanged copies. Customized or unknown
versions remain in place and receive review candidates under `skill-updates/`.
Neither command automatically overwrites project-owned method or business code.

## Configuration schema

The root document is strict TOML:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"
# Optional; create this file before enabling it.
# runtime_instructions = ".tactus/RUNTIME.md"

[limits]
max_concurrent_provider_calls = 4
check_timeout_seconds = 1800
script_timeout_seconds = 15300
plugin_timeout_seconds = 3600
provider_timeout_seconds = 13500
provider_outer_timeout_seconds = 14400
max_request_bytes = 1048576
max_frame_bytes = 33554432
max_stdout_bytes = 67108864
max_event_frames = 10000
max_stderr_bytes = 1048576
event_queue_bound = 4
native_max_line_bytes = 8388608
native_max_stdout_bytes = 1073741824
native_max_result_bytes = 4194304
native_max_stderr_bytes = 1048576
native_output_queue_bound = 8

[providers.codex]
command = ["tactus", "provider-host", "codex"]
model = "optional-provider-model"
effort = "high"

[providers.codex.options]
executable = "tools/codex"
timeout_seconds = 13440
extra_args = []

[effects."workspace.paths"]
command = ["tactus", "effect-host", "workspace-paths"]
observe_invocations = true

[effects."workspace.paths".options]

[plugins]
```

Unknown fields at the registry-definition level are rejected. Values inside
`options` are open JSON-compatible TOML values for the selected plugin.
Datetime values and non-finite floats are rejected because the configuration
must cross the JSON runtime boundary without changing meaning.

### Root fields

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `api` | string | yes | Must equal `clef.runtime/v1` |
| `default_provider` | string | yes | Registry name used by `invoke` and generation when no override is supplied |
| `instructions` | path string | yes | UTF-8 generation prompt; relative paths resolve from the workspace root |
| `runtime_instructions` | path string | no | UTF-8 instructions shared by business-task provider invocations; omitted means an empty prefix; relative paths resolve from the workspace root |
| `limits` | table | no | Workspace-owned deadlines, transport budgets, and provider concurrency; omitted fields use validated defaults |
| `daemon` | table | no | Optional monitoring provider, interval, deadline, retry budget and explicit retryable entries; activated by `run --daemon` |
| `providers` | table | yes in practice | Provider convenience registry; must contain `default_provider` |
| `effects` | table | no | Effect operations and optional invocation observers |
| `plugins` | table | no | Open generic plugin registry |

`instructions` belongs to the authoring agent used by `tactus generate`.
It is not prepended to provider calls made by the generated workflow. To share
business instructions across those calls, create a separate file and set
`runtime_instructions` to its path. The runtime JSON keeps its existing
`instructions` string field, populated from that optional file or `""`.

Existing configurations remain valid. Earlier runtimes also copied the
generation prompt into business calls; that unintended behavior is removed.
If an existing project relied on business guidance in `PROMPT.md`, move that
guidance into the explicitly configured runtime file. Initialization preserves
existing project-owned prompt files; update old generation guidance if it
still requires multiple stages or forbids compile checking.

### Daemon policy

| Field | Default | Meaning |
| --- | --- | --- |
| `provider` | `default_provider` | Independent provider registry key |
| `interval_seconds` | `60` | Delay between monitoring calls, 1–86400 seconds |
| `timeout_seconds` | `120` | Deadline per daemon provider invocation, 61–3600 seconds (60 seconds reserved for native cleanup) |
| `max_retries` | `1` | Maximum repairs/reruns per authorized entry, 0–10 |
| `retry_scripts` | `[]` | Workspace-relative paths explicitly authorized for idempotent repair/rerun |

The table configures policy; `tactus run --daemon` activates supervision for
one task. Unknown fields and invalid bounds are rejected. See
[provider setup](providers.md#optional-task-daemon) for a complete example and
the repair/escalation contract.

The standard `.tactus/segno.hs` organizer is an explicit-only exception to
numbered business entries. It can be selected by `check .tactus/segno.hs` or
`run --script .tactus/segno.hs`; default `run --all` does not include it.
Other control files and ordinary helper modules are not runnable entries.

### Provider definition

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `command` | non-empty string array | none | Executable followed by fixed arguments |
| `model` | string | omitted | Provider-specific model forwarded by the adapter |
| `effort` | string | omitted | Provider-specific reasoning effort or OpenCode variant |
| `options` | table | `{}` | Open adapter/plugin options |

### Effect definition

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `command` | non-empty string array | none | Executable followed by fixed arguments |
| `options` | table | `{}` | Open effect options |
| `observe_invocations` | boolean | `false` | Wrap other plugin calls with `observe.begin`/`observe.end` |

### Generic plugin definition

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `command` | non-empty string array | none | Executable followed by fixed arguments |
| `options` | table | `{}` | Open plugin options |

Names may appear in more than one namespace. An automatic lookup then fails as
ambiguous; use `provider:NAME`, `effect:NAME`, `plugin:NAME`, or the explicit
`--namespace` argument.

Provider-specific model and effort rules are maintained in [Provider setup](providers.md).

### Resource limits

All limit fields are optional as a group and individually; omitted values use
the defaults shown below. Tactus validates the nested deadlines and byte/queue
relationships before starting external work.

| Field | Default | Meaning |
| --- | ---: | --- |
| `max_concurrent_provider_calls` | `4` | Provider processes admitted by one Clef runtime |
| `check_timeout_seconds` | `1800` | One compiler/check process |
| `script_timeout_seconds` | `15300` | Outer deadline for one complete workflow entry |
| `plugin_timeout_seconds` | `3600` | Non-provider Clef plugin call |
| `provider_timeout_seconds` | `13500` | Tactus provider dispatch, outside the derived 13,440-second native deadline |
| `provider_outer_timeout_seconds` | `14400` | Clef provider supervisor, inside the workflow-script deadline |
| `max_request_bytes` | `1048576` | Encoded plugin request; hard host ceiling 16 MiB |
| `max_frame_bytes` | `33554432` | One plugin-v1 frame; also the 32 MiB ceiling |
| `max_stdout_bytes` | `67108864` | Cumulative plugin-v1 stdout drain |
| `max_event_frames` | `10000` | Observational event frames per invocation |
| `max_stderr_bytes` | `1048576` | Retained plugin stderr |
| `event_queue_bound` | `4` | Decoded frames waiting at each plugin handoff |
| `native_max_line_bytes` | `8388608` | One native provider event line |
| `native_max_stdout_bytes` | `1073741824` | Cumulative native provider stdout drain |
| `native_max_result_bytes` | `4194304` | Retained native result; 5 MiB ceiling |
| `native_max_stderr_bytes` | `1048576` | Retained native provider stderr |
| `native_output_queue_bound` | `8` | Native stdout lines waiting for parsing |

## Script discovery

Tactus recursively discovers regular `.hs` and `.lhs` files below
`.tactus/scripts` without following symlink directories.

A runnable entry name must match:

```text
NNN_lowercase_slug.hs
NNN_lowercase_slug.lhs
```

The prefix is exactly three digits. The non-empty slug contains lowercase ASCII
letters, digits, or single underscores and cannot begin or end with an
underscore. Examples:

```text
010_discover.hs       runnable, order 10
020_build_model.hs    runnable, order 20
Tactus/Shared.hs      helper only
draft.hs              helper only
```

Entries sort by numeric order, then by stable relative path. Helpers sort after
entries. `tactus check` and `tactus run` reject an empty selection: supply an
explicit path/`--script`, `--all`, or an inclusive `--from` / `--through`
range.

Explicit selection remains available:

```powershell
tactus check .tactus\scripts\Tactus\Shared.hs
tactus run `
  --script .tactus\scripts\020_build_model.hs `
  --script .tactus\scripts\030_review.hs
```

## Generation inputs

`tactus generate` combines:

- the natural-language goal;
- `PROMPT.md`;
- `.tactus/skills/tactus/SKILL.md` and its command/outcome references; and
- the selected provider configuration.

The provider is instructed to make the smallest sufficient source change.
Specialized supervision and Segno guidance is linked from the skills and read
when relevant; it is not all inserted into every generation prompt. Missing
Tactus generation guidance falls back to the binary's embedded copy.
It may use `tactus check` with explicit source paths for compiler feedback,
but must not execute the generated business workflow or its external effects.
Changes to helper modules count as generation output; a new numbered entry is
not required. Empty or unchanged sources do not count. Tactus discovers the
resulting files and reports the delta. A successful generation report alone
does not prove that sources compile or that the workflow meets its goal.

## Workspace path observer

The built-in `workspace.paths` effect records bounded path metadata before and
after observed plugin invocations. It excludes `.git`, path-effect state, run
journals, Tactus Cabal output, and nested `target`, `node_modules`, `build`, and
`dist-newstyle` directories. Individual paths that cannot be safely resolved or
read are reported as warnings and omitted; they are not access-control errors
for the provider invocation.

The effect is diagnostic, content-free evidence. It is not attribution,
backup, rollback, or a complete filesystem audit.

## Validate changes

After editing configuration or moving a checkout:

```powershell
tactus doctor
tactus runtime-json
tactus smoke
tactus list
```

`runtime-json` contains resolved business-task instructions and command paths. Use it
for local diagnosis; do not commit or paste it without reviewing sensitive
content. Machine consumers needing a redacted workspace view can use
`tactus studio inspect`. Current Motivo HTML reports display method-generated
snapshots and do not call that API.
