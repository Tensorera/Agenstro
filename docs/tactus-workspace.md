---
title: Tactus workspace and configuration
status: alpha
last_verified: 2026-08-20
applies_to: "tactus-runtime 0.3.0"
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
  runs/
  sessions/
  skills/
    tactus/
      SKILL.md
      references/
  path-effect/
  dist-newstyle/
  segno/
```

| Path | Owner and lifecycle |
| --- | --- |
| `tactus.toml` | Project-owned typed registry; edit deliberately and version it when appropriate |
| `cabal.project` | Init-generated link to Clef; repair it after moving the SDK |
| `PROMPT.md` | Project-owned instructions included in workflow-generation requests |
| `scripts/` | Project-owned Haskell entries and helper modules |
| `skills/tactus/` | Init-materialized agent guidance; missing files fall back to the embedded copy |
| `runs/` | Runtime-owned bounded diagnostic journals; not replay state |
| `sessions/` | Runtime-owned current elicitation state and append-only decision evidence |
| `path-effect/` | Runtime-owned snapshot/observation tokens |
| `dist-newstyle/` | Rebuildable Cabal output |
| `segno/` | Segno-owned job, trigger, lifecycle, and business-state data |

Do not edit runtime-owned state while a command is active. Back up project-owned
files, session evidence, and Segno databases before deleting `.tactus`; see
[Workspace operations](operations.md).

`sessions/` is additive for existing workspaces: `session list` returns an
empty list when the directory has not been created yet, while a later
idempotent `tactus init` creates it. Motivo and other clients use
`tactus session list/show/answer`; they do not read or write this directory.
See the [session control API](reference/session-control-v1.md).

## Configuration schema

The root document is strict TOML:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus", "provider-host", "codex"]
model = "optional-provider-model"
effort = "high"

[providers.codex.options]
timeout_seconds = 1800
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
| `providers` | table | yes in practice | Provider convenience registry; must contain `default_provider` |
| `effects` | table | no | Effect operations and optional invocation observers |
| `plugins` | table | no | Open generic plugin registry |

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
entries. `tactus run` without `--script` executes runnable entries only;
`tactus check` without explicit paths checks every discovered source.

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
- `.tactus/skills/tactus/SKILL.md` and its two references; and
- the selected provider configuration.

The provider is instructed to write Haskell sources but not to run Cabal, GHC,
tests, or workflows during generation. Tactus discovers the resulting files
and reports the delta. It never treats generation as proof that scripts compile
or are safe.

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

`runtime-json` contains resolved internal instructions and command paths. Use it
for local diagnosis; do not commit or paste it without reviewing sensitive
content. Motivo uses `tactus studio inspect` instead of reading this document.
