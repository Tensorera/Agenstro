---
title: Program workflows with Clef
status: alpha
last_verified: 2026-08-20
applies_to: "clef-sdk 0.3.0.0"
---

# Program workflows with Clef

Clef is the typed Haskell authoring surface of Agenstro. Use this guide to
compose provider tasks, effects, arbitrary plugins, parallel work, guards, and
recoverable workflow failures.

## The core type

`Workflow a` means “a workflow that returns an `a` when it succeeds.” It is an
ordinary `Functor`, `Applicative`, and `Monad`, so normal Haskell `do` notation
defines sequencing and GHC checks every value passed between steps.

```haskell compile
workflow :: Workflow Int
workflow = do
  left <- pure 19
  right <- pure 23
  requireBecause "sum must be positive" (> 0) (left + right)
```

The constructor is intentionally hidden. Use the public combinators rather
than creating a second interpreter or serializing Haskell continuations.

## Provider tasks

A `Task input output` renders an input as a provider prompt and decodes the
provider's final text into an output.

```haskell
{-# LANGUAGE DeriveAnyClass #-}
{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE OverloadedStrings #-}

import Clef
import Data.Aeson (FromJSON, ToJSON)
import Data.Text (Text)
import GHC.Generics (Generic)

data Plan = Plan
  { summary :: Text,
    files :: [FilePath]
  }
  deriving (Show, Generic, FromJSON)

planTask :: Task Text Plan
planTask = jsonTask "plan" $ \request ->
  "Return JSON with summary and files for this request: " <> request

workflow :: Workflow Plan
workflow = invoke planTask "Add an import pipeline"
```

Choose the constructor by result format:

| Constructor | Result |
| --- | --- |
| `textTask` | Provider final text unchanged |
| `jsonTask` | Strict JSON decoded with `FromJSON` |
| `task` | Custom `Text -> Either Text output` decoder |

`invoke` uses `default_provider` from `.tactus/tactus.toml`. Use `invokeWith`
for a typed, per-call override:

```haskell
reviewer :: ProviderRef
reviewer =
  (providerRef "opencode")
    { providerRefModel = Just "provider-specific-model",
      providerRefEffort = Just "high"
    }

review <- invokeWith reviewer reviewTask plan
```

Provider names, models, effort strings, options, and extra arguments remain
open runtime values. Clef checks Haskell value wiring; the selected adapter
decides whether a particular model or effort is valid. See [Provider setup](providers.md).

## Effects

An `Operation output` calls a named method in the `[effects]` registry. The
request and result are JSON-encoded but the Haskell result remains typed:

```haskell
data Lookup = Lookup { path :: FilePath }
  deriving (Generic, ToJSON)

data Metadata = Metadata { size :: Int }
  deriving (Generic, FromJSON)

metadata :: Operation Metadata
metadata = operation "filesystem.metadata" "read" (Lookup "input.dat")

workflow :: Workflow Metadata
workflow = perform metadata
```

Use the provided `Clef.Effect.WorkspacePaths` wrappers for the built-in
observational path effect. Effects are capabilities, not rollback transactions;
ordinary Haskell `IO` is also possible through `liftIO` and is not intercepted.

## Arbitrary plugins

`Plugin input output` exposes the open `[plugins]` registry without pretending
every plugin is a provider or effect:

```haskell
data Add = Add { left :: Int, right :: Int }
  deriving (Generic, ToJSON)

data Sum = Sum { value :: Int }
  deriving (Generic, FromJSON)

calculator :: Plugin Add Sum
calculator = jsonPlugin "calculator" "add"

workflow :: Workflow Sum
workflow = call calculator (Add 19 23)
```

Use `rawPlugin` only when the workflow intentionally works with untyped Aeson
`Value`. Prefer `jsonPlugin` at application boundaries so invalid results fail
before they enter later steps.

## Explicit concurrency

Clef never infers a hidden DAG. `parallel` and `parallelAll` are the only
combinators that request concurrency:

```haskell
(security, correctness) <-
  parallel
    (invoke securityReview plan)
    (invoke correctnessReview plan)
```

Both branches share the runtime and may perform external work. A failure in
one branch does not imply the other branch or its side effects were rolled
back.

## Guards and recoverable failures

Use `require` for a predicate and `requireBecause` when a human explanation is
important:

```haskell
approvedPlan <- requireBecause "review rejected the plan" approved plan
```

`attempt` catches `WorkflowError` without swallowing asynchronous exceptions:

```haskell
result <- attempt (invoke optionalTask input)
case result of
  Right value -> pure value
  Left workflowError -> fallback workflowError
```

Important error groups include:

- invalid runtime configuration or unknown registry names;
- provider/effect/plugin process or protocol failure;
- typed result decode failure;
- an explicit failed requirement;
- a plugin-reported failure; and
- `PluginOutcomeUnknown`, where external work may have happened but no
  trustworthy terminal result was received.

Never automatically retry `OutcomeUnknown` unless the external operation is
known to be idempotent and has been reconciled.

## Typed norms, rubrics, and refinement

`Norm artifact` attaches one stable convention to the artifact type it
constrains. A rubric composes norms of the same type and projects them in two
directions: bounded guidance for generation and a typed critique after
generation.

```haskell
{-# LANGUAGE OverloadedStrings #-}

import Clef
import Data.Text (Text)

newtype LaTeX = LaTeX { unLaTeX :: Text }

uprightDifferential :: Norm LaTeX
uprightDifferential =
  (norm
    (NormId "math.notation.upright-differential")
    "Use an upright differential operator."
    Style)
    { normGuidance = Just "Write \\,\\mathrm{d}x in integrands.",
      normCheck = Just (SpecCheck unLaTeX (Absence "(?<!\\\\mathrm\\{)\\bd(?=[a-zA-Z]\\b)" False)),
      normProvenance = Authored "project"
    }

articleRubric :: Rubric LaTeX
articleRubric = rubric [uprightDifferential]

checkArticle :: LaTeX -> Workflow Critique
checkArticle =
  judgeWith
    ( NormChecker
        { normCheckerPlugin = "latex-norm-check",
          normCheckerArtifact = "article.tex"
        }
    )
    articleRubric
```

Serializable `CheckSpec` values are sent to an ordinary configured plugin.
The repository reference checker can be registered as a generic plugin:

```toml
[plugins.latex-norm-check]
command = ["python", "D:/src/Agenstro/plugins/latex-norm-check/latex_norm_check.py"]
```

`judge` uses the conventional plugin name `norm-check`; use `judgeWith` when a
workspace registers another name or artifact label. A `NativeCheck` remains
available for a typed Haskell check. The `Occurrence` check-spec constructor
shares its name with the Segno occurrence type, so import it qualified from
`Clef.Norm` when using the umbrella `Clef` module.

Every `Critique` lists `checked` and `unchecked` norm identities separately.
No violations plus unchecked norms means only that the available checks found
nothing; it is not a blanket quality claim. `refine` and `refineWith` bound the
generate/judge/repair loop and pass the previous structured critique to the
next generation. Unknown plugin outcomes propagate rather than becoming an
automatic repair round. Reaching the round limit returns the last candidate and
critique even if the policy still rejects it, so the caller remains responsible
for the delivery gate. `refineBudget` is explicit generator policy; the
combinator cannot inject it into a caller-defined generator automatically.

See the [norm v1 reference](reference/norm-v1.md) and
[ADR-0005](adr/0005-norms-rubrics-and-refinement.md).

## Running a workflow

For a normal Tactus script:

```haskell
main :: IO ()
main = do
  result <- runTactus workflow
  print result
```

Tactus supplies the runtime configuration through the process environment.
`runTactus` renders expected errors as one bounded human message and exits
without a Haskell call stack. Use lower-level runners when embedding Clef:

| Runner | Use |
| --- | --- |
| `runTactus` | Normal script entry; human presentation and process exit |
| `runTactusWithRecords` | Return `Either WorkflowError value` plus runtime records |
| `runWorkflow` | Execute against an explicitly constructed `Runtime` |

`newRuntimeWithSink` accepts an `EventSink` for an embedding that needs every
runtime record. A sink is an observation channel: degradation is recorded and
does not change a provider's authoritative terminal result.
Long-lived embedders should acquire runtimes with `withRuntime` or
`withRuntimeWithSink`; both boundedly flush and stop the sink worker on normal
return, exceptions, and cancellation. If manual ownership is necessary,
`closeRuntime` is idempotent.

## State transitions and presentation

Clef distinguishes a genuine transition from a message. A
`RuntimeStateTransition` contains:

- `state_before`;
- the trigger (`request`, `event`, `timer`, `internal_result`, or `control`);
- the guard condition, decision, and reason; and
- `state_after`.

Human output is limited to `[state]`, `[info]`, `[warning]`, and `[error]` plus
natural language. Raw provider frames remain diagnostic records and do not
become workflow values or fill the normal terminal display. The complete
contract is in [Logs and run evidence](observability.md).

## Persistent tasks

Clef also exports the typed value boundary used by Segno:

- `Trigger state event` and `triggerSource`;
- `mapTrigger`, `filterTrigger`, `mergeTrigger`, and `gate`;
- versioned `State state` and `checkpoint`;
- `Occurrence event`;
- `PersistentTask state event output`; and
- decisions `Ignore`, `Complete`, `Retry`, and `Fail`.

Segno owns clocks, cursors, leases, attempts, and durable SQLite data. Start
with [Segno persistent tasks](segno.md); do not put scheduling loops into Clef.

## Complete example and API source

The repository example at `clef-sdk/haskell/examples/TypedWorkflow.hs` combines
typed JSON tasks, two parallel reviews, requirements, and the workspace path
effect. The public exports are defined by `Clef`, while protocol details remain
in the [local plugin protocol](reference/plugin-protocol-v1.md).
