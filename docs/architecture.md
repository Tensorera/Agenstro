---
title: Agenstro 0.3 architecture
status: alpha
last_verified: 2026-08-15
applies_to: "Clef 0.3.0.0 and Tactus 0.3.0"
platforms: [windows, ubuntu]
---

# Agenstro 0.3 architecture

Agenstro 0.3 has two authoritative runtime components: Clef is a typed Haskell
embedded domain-specific language (EDSL), and Tactus is a small Python CLI that
creates project-local configuration and delegates compilation or execution to
the Haskell toolchain. Provider and effect behavior lives behind replaceable
one-shot local plugins.

This is a source-alpha architecture for trusted local development. It does not
contain a daemon network, database-backed workflow service, permission broker,
artifact store, rollback engine, or adversarial-code sandbox.

## Component ownership

| Component | Owns | Deliberately does not own |
| --- | --- | --- |
| Clef `0.3.0.0` | `Workflow a`, typed `Task input output`, typed `Operation output`, provider/effect invocation, explicit `parallel`, requirements, runtime records | A custom language parser, global DAG, daemon, plugin discovery service, sandbox, credentials, artifact/CAS policy |
| Tactus `0.3.0` | `.tactus` layout, TOML loading, runtime JSON generation, script discovery, prompt generation, Cabal/GHC/runghc invocation, diagnostics, plugin smoke probes | Python cells, notebooks, durable scheduler, database, checkpoint/rollback, GUI, provider authentication |
| Provider plugins | Translate the open JSON request into a native coding-agent CLI invocation and normalize its result | Core workflow semantics, effect policy, cross-provider permission equivalence |
| Effect plugins | Perform or observe an external operation through a separately named registry | Provider output, authorization, automatic rollback |
| Haskell workflow | Ordinary program control flow and any direct `IO` chosen by its author | Automatic confinement merely because some operations use Clef |

Motivo Studio and Segno Flow remain in the repository as frozen 0.2 evidence.
They are outside this component graph and outside the 0.3 release gate.

## End-to-end topology

```text
project author or coding agent
        |
        | writes NNN_slug.hs / .lhs
        v
  .tactus/scripts
        |
        | tactus check / tactus run
        v
Tactus 0.3 (Python CLI)
        |-- reads .tactus/tactus.toml and PROMPT.md
        |-- writes .tactus/runtime.json (clef.runtime/v1)
        |-- builds clef-sdk through Cabal
        `-- invokes GHC -fno-code or runghc
                         |
                         v
                  Clef 0.3 Haskell EDSL
                         |
              Task invoke / Operation perform
                         |
                         v
              agenstro.plugin/v1 process
                  |                    |
                  v                    v
           provider registry      effect registry
         Codex / Claude Code /   workspace.paths
               OpenCode
```

`tactus generate` takes a shorter route through the same configured provider
and observer plugins: Tactus assembles `PROMPT.md` plus the requested goal,
starts the provider adapter, lets it edit the project, records observer
evidence, and lists resulting scripts. Generation does not compile or run them.

## Clef's type boundary

A Clef workflow is ordinary Haskell:

```haskell
Task input output
Operation output
Workflow result

invoke     :: Task input output -> input -> Workflow output
invokeWith :: ProviderRef -> Task input output -> input -> Workflow output
perform    :: Operation output -> Workflow output
parallel   :: Workflow a -> Workflow b -> Workflow (a, b)
```

GHC can reject a mismatch between the output of one task and the input of the
next. It cannot prove that a prompt is semantically correct, that a provider is
honest, that a plugin terminates, or that arbitrary `IO` is safe. Dynamic
provider and effect names are resolved from runtime configuration.

Sequential `do` notation is the default execution order. `parallel` is the
only core primitive that requests concurrency. Clef does not first compile the
program into a global workflow graph, so ordinary branches and values retain
normal Haskell semantics.

## Tactus workspace and configuration flow

`tactus init` creates only missing files:

```text
project/
  .tactus/
    tactus.toml
    cabal.project
    PROMPT.md
    scripts/
```

At `generate`, `check`, `run`, or `smoke` time, Tactus validates
`tactus.toml` and writes `.tactus/runtime.json`. The runtime document contains:

- API name `clef.runtime/v1`;
- the absolute project workspace;
- a default provider name;
- separate provider and effect registries;
- direct command argument arrays, models, effort/variant values, and open
  plugin options; and
- the contents of `PROMPT.md` as workflow instructions.

Tactus sets `TACTUS_RUNTIME_CONFIG` to the absolute runtime JSON path before a
Haskell program starts. `runTactus` reads that file. Command arrays are executed
without shell parsing, but the executable itself is still arbitrary trusted
local code.

Runnable scripts use `NNN_slug.hs` or `NNN_slug.lhs`. Discovery sorts by the
three-digit number and then relative path. Other Haskell files are helpers:
`check` includes them by default, while `run` ignores them unless selected
explicitly. Both commands are fail-fast unless `--keep-going` is supplied.

## One-shot plugin protocol

Providers and effects share `agenstro.plugin/v1`, but their registries and
result meanings remain separate. Each call:

1. starts one configured executable in the workflow workspace;
2. inherits the caller's environment;
3. writes one UTF-8 JSON request and closes standard input;
4. accepts zero or more correlated JSONL event frames;
5. requires exactly one correlated terminal result; and
6. treats standard error as human diagnostics rather than protocol data.

```json
{"api":"agenstro.plugin/v1","id":"...","method":"invoke","params":{}}
{"type":"event","id":"...","event":{"type":"provider.raw"}}
{"type":"result","id":"...","ok":true,"value":{}}
```

Malformed JSON, duplicate object keys, correlation mismatch, duplicate or
missing terminal results, data after a terminal result, invalid numeric values,
and inconsistent process exits are failures. Provider invocation failures may
be reported as `outcome_unknown` when an external request could have completed
before the local process failed.

Version 1 is deliberately one-shot. It has no persistent plugin session,
authentication handshake, socket discovery, default deadline, output quota, or
live UI stream. Clients preserve event order but currently buffer protocol
output until the plugin exits. See the
[protocol reference](reference/plugin-protocol-v1.md) for the complete frame and
method contract.

## Bundled provider adapters

The reference adapters favor non-interactive native CLI modes so a generated
workflow does not stop on an approval prompt:

| Registry name | Native invocation shape | Important boundary |
| --- | --- | --- |
| `codex` | `codex exec --dangerously-bypass-approvals-and-sandbox --json ...` | Uses an ephemeral execution and skips the Git-repository check; the native CLI still owns credentials, model access, and endpoint behavior |
| `claude-code` | `claude -p --dangerously-skip-permissions --output-format stream-json ...` | Uses no session persistence; the native CLI still owns authentication and service policy |
| `opencode` | `opencode run --auto --format json ...` plus inline `permission=allow` | `--auto` approves ask decisions but cannot override an explicit deny or managed configuration, so a full bypass is not claimed |

Model identifiers remain open strings. Clef/Tactus map the configured effort
to Codex/Claude `effort` and to OpenCode `variant`; they do not define a closed
cross-provider equivalence. Options such as `extra_args`, `extra_env`,
`command_prefix`, and `timeout_seconds` are plugin concerns and can change
native behavior materially.

The adapters and protocol edge cases are tested with local fakes in CI. CI does
not install provider CLIs, authenticate accounts, contact model endpoints, or
certify a provider CLI version. Live acceptance is explicitly local.

## `workspace.paths` observation effect

The bundled effect can create opaque snapshots, compare two snapshots, forget
snapshot state, and wrap provider invocation with `observe.begin` /
`observe.end`. It compares path kind, size, and SHA-256 content digest and
returns workspace-relative added, modified, deleted, and type-changed paths.

Configured observers begin in deterministic name order and end in reverse
order. Cleanup continues after an observer failure, but cancellation cleanup is
best effort rather than an exactly-once transaction.

The effect excludes `.git`, its own `.tactus/path-effect` state, and
`.tactus/dist-newstyle`. It does not apply general `.gitignore` rules. It does
not retain file contents, block changes, infer authorization, restore files, or
attribute a change conclusively to one process. Final snapshots cannot observe
reads or transient writes, and concurrent writers make attribution ambiguous.

## Process, trust, and failure boundaries

- Haskell workflows, plugin commands, and native provider CLIs execute with the
  current user's operating-system authority and inherited environment.
- Passing an argument array without a shell avoids shell parsing; it does not
  authenticate or sandbox the executable.
- `check` is a compile check, not a proof that running the program is safe.
- `generate`, `smoke --live`, and provider calls made during `run` can consume
  quota and modify external state.
- A normal provider invocation has no framework-imposed timeout. A configured
  `options.timeout_seconds` bounds the direct child wait but is not a strong
  process-tree cancellation guarantee.
- `liftIO` intentionally permits work outside the plugin protocol and outside
  `workspace.paths` evidence.

These are documented alpha boundaries, not missing enforcement that callers
should assume exists elsewhere.

## Frozen 0.2 surfaces

The Clef/Tactus Rust product cores were removed after the `c679f45` snapshot.
Python archives, the shared foundation, scheduler, Electron, and historical
documentation remain for migration and design evidence. For 0.3:

- Motivo Studio is frozen and is not a supported GUI for Haskell workflows;
- Segno Flow is frozen and makes no replay guarantee for arbitrary Haskell `IO`
  or live provider calls;
- the Clef Python builder and Tactus worker/Jupyter sources under `archive/`
  are not installed artifacts;
- the old Clef/Tactus Rust implementations are available only through Git
  history; and
- checkpoint, CAS, artifact publication, database, daemon, and rollback claims
  from 0.2 do not carry into 0.3.

Any revival must be an explicit later design decision. The current direction is
recorded in the [roadmap](roadmap.md) and the
[0.2 to Haskell 0.3 migration guide](migrations/0.2-to-haskell-0.3.md).
