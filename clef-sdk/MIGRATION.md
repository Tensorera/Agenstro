# Clef 0.2 to Haskell 0.3 migration

Clef `0.3.0.0` starts a new Haskell execution path. The previous Python builder
and tests moved to `../archive/clef-sdk-python-0.2/`. The old Rust product core
was removed from the current tree and can be inspected in Git history at
`c679f45`; retained shared Protobuf/foundation code is not the authoritative
model for new workflow code.

## What changes

| Clef 0.2 | Clef 0.3 candidate |
| --- | --- |
| Python `Workflow`/`Task` builders | Ordinary Haskell scripts using `Workflow a` and `Task i o` |
| Rust workflow specification and DAG compiler | Haskell `do` order, with explicit `parallel` only |
| Closed effort and capability enums | Open `Text` model/effort/options passed to adapters |
| Daemon-owned single backend | Provider selected by `ProviderRef` and runtime JSON |
| Artifact bindings, publication gates, and CAS | Not part of the core |
| Provider-reported file changes | Independent observer effect evidence |
| Contract-only adapter manifests | One-shot `agenstro.plugin/v1` JSONL subprocesses with incremental events |

The old one-way `convert_legacy_workflow` function still exists in the frozen
Python tree for callers that must inspect or preserve a `clef.workflow/v2`
value. Its output is not an input format for the Haskell EDSL. A dynamic
workflow should instead be rewritten as a normal Haskell module so GHC can
check task input/output and operation result wiring.

## Minimal migration

1. Create a Haskell `Main` module and import `Clef`.
2. Replace each agent step with a `textTask` or `jsonTask` value.
3. Chain tasks with `invoke` or `invokeWith`; the result type of one task must
   match the input type of the next.
4. Replace external operations with typed `Operation a` values. Workspace path
   helpers live in `Clef.Effect.WorkspacePaths`.
5. Use `Plugin i o` plus `jsonPlugin`/`call` for application-defined plugins
   that are neither providers nor effects.
6. Put provider/effect/plugin command arrays in a `clef.runtime/v1` JSON file and set
   `TACTUS_RUNTIME_CONFIG` before invoking the compiled script.
7. Use `parallel` only where the script explicitly wants concurrent execution.

For example:

```haskell
workflow :: Workflow Review
workflow = do
  plan <- invoke planTask request
  review <- invokeWith (providerRef "claude-code") reviewTask plan
  requireBecause "review rejected the plan" approved review

main :: IO ()
main = runTactus workflow >>= print
```

## What is not promised

The Haskell type checker verifies ordinary Haskell types; it does not verify a
prompt's meaning, provider honesty, plugin installation, filesystem behavior,
termination, authorization, or safety. Those are runtime facts or future
plugin concerns. `liftIO` remains available and can perform effects outside the
provider/effect protocol.
