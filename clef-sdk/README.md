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

## Norms, rubrics, and refinement

`Clef.Norm` defines typed, stable domain conventions and the open
`agenstro.norm/v1` wire format. `Clef.Rubric` composes those conventions,
selects bounded generation guidance, reports both checked and unchecked norms,
and supports a caller-bounded generate/judge/repair loop. Serializable checks
are normally batched through an ordinary `norm-check` plugin; `NativeCheck`
remains the explicit Haskell escape hatch.

`validationFailures` projects those same Norm violations into structured gate
records containing validator stage, rule, expected, observed, and provenance.
`validate`/`validateWith` can gate at `Correctness` or `Blocking`; they raise
`ValidationFailed` without introducing another checker or changing either wire
protocol.

The umbrella `Clef` module re-exports this API except the `Occurrence`
check-spec constructor, whose name is already used by Segno. Import
`Clef.Norm` qualified when constructing an occurrence check. The full contract
and checker configuration example are in the [Clef guide](../docs/clef.md).

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
    "norm-check": {
      "command": ["python", "D:/src/Agenstro/plugins/latex-norm-check/latex_norm_check.py"],
      "options": {}
    },
    "calculator": {
      "command": ["calculator-plugin"],
      "options": {}
    }
  },
  "instructions": "Instructions prepended to every provider prompt.",
  "limits": {
    "max_concurrent_provider_calls": 4,
    "plugin_timeout_seconds": 3600,
    "provider_timeout_seconds": 13500,
    "provider_outer_timeout_seconds": 14400,
    "max_request_bytes": 1048576,
    "max_frame_bytes": 33554432,
    "max_stdout_bytes": 67108864,
    "max_event_frames": 10000,
    "max_stderr_bytes": 1048576
  }
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

`limits` is also optional. Missing fields retain the previous Clef defaults.
When `loadRuntimeConfigFromEnv` sees a provider command containing the Tactus
`dispatch` subcommand, it adds the configured `provider_timeout_seconds`
(13,500 seconds by default) unless either explicit timeout form is already
present. The dispatcher gives the native provider CLI 13,440 seconds by
default, retaining 60 seconds to reap it and deliver its terminal frame. Clef
uses `provider_outer_timeout_seconds` (14,400 seconds) for the outer provider
supervisor; the enclosing Tactus workflow script has a separate 15,300-second
outer deadline. Ordinary effects and plugins use `plugin_timeout_seconds`.
The remaining fields configure the outer plugin-v1 transport budgets;
`max_frame_bytes` is capped at 33,554,432 bytes.

One runtime admits at most `max_concurrent_provider_calls` provider processes.
Permits surround only direct provider boundaries and are released on every
exit path. `parallelAllBounded` provides an independent bound for arbitrary
workflow branches while retaining input order and structured cancellation.

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
`runWorkflow` boundary. Under load, low-priority plugin events may be dropped
with a `runtime.sink_degraded` record and `events_dropped` count while terminal
records retain reserved capacity. A blocked or failed sink is retained as a
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
sequential `do` semantics; concurrency happens only through `parallel`,
`parallelAll`, or `parallelAllBounded`. There is
no type-level effect row, pre/post typestate, global DAG, scheduler, artifact
store, authorization layer, or sandbox in the core.

`liftIO` is intentionally available. It does not create a security boundary and
Clef does not claim to intercept arbitrary IO performed by a Haskell script.
Configured observer effects wrap provider invocations and return evidence
separately from provider results.
