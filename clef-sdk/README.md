# Clef SDK

Clef `0.3.0.0` is a small, typed Haskell EDSL for dynamic coding-agent
workflows. A workflow is an ordinary Haskell program: GHC checks the values
passed between `Task input output`, `Operation output`, and arbitrary `Plugin
input output` calls. Runtime provider, effect, and plugin availability stays
dynamic.

The canonical user guide is [Program workflows with Clef](../docs/clef.md);
this component README stays focused on package development and local examples.

The Haskell implementation under `haskell/` is authoritative for new
development. The Python and Rust `0.2` implementations were removed from the
current tree and remain available in Git history (including snapshots
`3eef756` and `c679f45`). Neither is a dependency or release artifact of the
Haskell package.

Clef is licensed with the rest of Agenstro under
[GNU AGPL v3.0 only](../LICENSE).

## Build and test

From the repository root:

```powershell
$env:PATH = 'C:\ghcup\bin;C:\cabal\bin;' + $env:PATH
cabal build --builddir=Build/cabal all
cabal test --builddir=Build/cabal all --test-show-details=direct
```

The package uses GHC2021 and has no custom parser or code generator. See
[`haskell/examples/TypedWorkflow.hs`](haskell/examples/TypedWorkflow.hs) for a
script that chains two typed tasks, runs reviews in parallel, and calls the
typed workspace-path effect.

The dependency set is intentionally conventional: Cabal/GHC for the project
and static checks, `aeson` for the open JSON boundary, `async` for explicit
structured concurrency, and `process` for direct argv subprocesses. We do not
add an extensible-effects framework in the first core; plugin-specific typed
wrappers can grow independently without turning every workflow into a type-
level effect row. The upstream references are the [GHCup toolchain
guide](https://www.haskell.org/ghcup/), [Cabal project
guide](https://cabal.readthedocs.io/en/stable/cabal-project-description-file.html),
[`aeson`](https://hackage.haskell.org/package/aeson), and
[`async`](https://hackage.haskell.org/package/async).

`textTask` and `jsonTask` cover the common provider result shapes. The general
`task` constructor accepts any `Text -> Either Text output` decoder while still
keeping the task's input and output wiring visible to GHC.

`jsonPlugin name method` creates a general `ToJSON input => FromJSON output`
boundary and `rawPlugin` is the explicit `Value -> Value` escape hatch. Calls
remain ordinary typed workflow steps:

```haskell
let add = jsonPlugin "calculator" "add" :: Plugin (Int, Int) Int
answer <- call add (19, 23)
```

`attempt` catches only `WorkflowError` inside the EDSL, allowing typed fallback
or post-failure effect calls without swallowing cancellation. For callers that
need evidence after failure, `runTactusWithRecords` returns the workflow outcome
and the accumulated provider/event/effect records; `runTactus` remains the
smallest convenience runner.

## Runtime configuration

`runTactus` reads the path in `TACTUS_RUNTIME_CONFIG`. The referenced file is
JSON with this schema:

```json
{
  "api": "clef.runtime/v1",
  "workspace": "D:\\absolute\\workspace",
  "default_provider": "codex",
  "providers": {
    "codex": {
      "command": ["codex-provider-adapter"],
      "model": "optional-open-string",
      "effort": "optional-open-string",
      "options": {}
    }
  },
  "effects": {
    "workspace.paths": {
      "command": ["workspace-paths-effect"],
      "options": {},
      "observe_invocations": true
    }
  },
  "plugins": {
    "calculator": {
      "command": ["calculator-plugin"],
      "options": {}
    }
  },
  "instructions": "Instructions prepended to every provider prompt."
}
```

`plugins` is optional and defaults to `{}` for older runtime configurations.
It is an independent, open registry: general plugins do not pretend to be
providers or effects. Clef adds runtime-owned `workspace` and `options` fields
to each general plugin request just as it does for effect calls.

Commands are argument arrays and are executed directly, never through a shell.
They run with the configured workspace as their current directory and inherit
the caller's environment. Provider-specific approval-bypass flags, models,
effort mapping, and credentials belong to provider adapters, not the EDSL.

## Plugin boundary

Each provider, effect, or general plugin call starts one subprocess and writes
one JSONL request. Stdout is decoded incrementally by LF while stdin and stderr
are drained concurrently. The request is:

```json
{"api":"agenstro.plugin/v1","id":"clef-1","method":"invoke","params":{}}
```

The plugin may write zero or more correlated `event` lines and must write
exactly one terminal line:

```json
{"type":"event","id":"clef-1","event":{"type":"provider.raw","text":"..."}}
```

Event subtype values are open so adapters can add diagnostics without changing
the core protocol. The terminal line is:

```json
{"type":"result","id":"clef-1","ok":true,"value":{}}
```

or:

```json
{"type":"result","id":"clef-1","ok":false,"error":{"code":"...","message":"..."}}
```

Clef rejects malformed JSON, correlation mismatches, missing or repeated
terminal results, data after a terminal result, and non-zero exits that do not
accompany a valid structured failure.
Plugin events are retained as runtime records and delivered to `EventSink` as
soon as each complete line arrives, before the terminal result when applicable;
they are not part of `Workflow`'s return type. `newRuntime` keeps the default
stderr projection, while `newRuntimeWithSink config (EventSink handler)` installs
a caller-defined projection without losing in-memory records. Clef serializes
the handler on a bounded worker queue and boundedly flushes it at the
`runWorkflow` boundary; a blocked or failed sink is retained as a
`runtime.sink_failed` internal diagnostic without changing a successful value
or known plugin failure. Typed general-plugin terminal values
are retained as `PluginValueRecord` before
decoding, so evidence survives a result-schema error. Stderr
diagnostics are drained concurrently and recorded after process completion. A
valid structured `ok:false` result remains a plugin-reported
failure even when an adapter also exits non-zero. Provider values and effect
evidence have distinct record types.

## Deliberate boundaries

`Workflow a` is an abstract wrapper around `Runtime -> IO a`. It has normal
sequential `do` semantics; concurrency happens only through `parallel`. There is
no type-level effect row, pre/post typestate, global DAG, scheduler, artifact
store, authorization layer, or sandbox in the core.

`liftIO` is intentionally available. It does not create a security boundary and
Clef does not claim to intercept arbitrary IO performed by a Haskell script.
Configured observer effects wrap provider invocations and return evidence
separately from provider results.
