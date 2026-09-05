---
title: Program workflows with Clef
status: alpha
owners: [clef]
last_verified: 2026-09-01
applies_to: "clef-sdk 0.3.0.0"
platforms: [windows, ubuntu]
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

Clef never infers a hidden DAG. `parallel`, `parallelAll`, and
`parallelAllBounded` are the combinators that request concurrency:

```haskell
(security, correctness) <-
  parallel
    (invoke securityReview plan)
    (invoke correctnessReview plan)
```

Use a bounded collection when the input can grow:

```haskell
reviews <- parallelAllBounded 4 (reviewOne <$> documents)
```

`parallelAllBounded` preserves traversal order even when branches finish out
of order. Like `parallelAll`, it uses structured cancellation: if one branch
throws, unfinished siblings are cancelled. The limit must be positive.

The runtime also admits at most `limits.max_concurrent_provider_calls`
provider processes at once (default 4). That shared permit is acquired at each
`provider:` plugin boundary and released on completion, failure, timeout, or
cancellation. Clef does not hold it while evaluating an enclosing or nested
`Workflow`, so nested composition cannot consume a permit merely by waiting
for another branch. Generic plugins and effects are not charged to this
provider-agent pool; use `parallelAllBounded` to bound broader work.

Tactus may include this optional object in `clef.runtime/v1` runtime JSON:

```json
{
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

`provider_timeout_seconds` is the Tactus dispatch deadline (13,500 seconds by
default); the dispatcher derives a 13,440-second native-provider deadline so
it retains 60 seconds for cleanup. `provider_outer_timeout_seconds` is Clef's
14,400-second outer process boundary, and the enclosing workflow script has a
separate 15,300-second outer deadline. Missing `limits` or fields retain safe
defaults. Byte and frame limits still govern the outer
`agenstro.plugin/v1` stream; `max_frame_bytes` cannot exceed 33,554,432 bytes,
and native provider streaming has separate adapter-local limits.

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

The structured cause of a Clef `PluginOutcomeUnknown` includes a safe summary:
`phase`, `frames_seen`, progress counts, the last event **type** (never its
prompt or model-output body), and `last_event_unix_ms`, timestamped when Clef
accepted that event rather than when it later formed the diagnostic. It also
sets `external_effect_possible` and a reconciliation policy that marks blind
automatic retry unsafe. Provider-supplied detail objects are not copied into
this broadly consumed diagnostic; `reported_details_withheld` records whether
one was present.

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

Use the same rubric as an explicit delivery gate:

```haskell
validated <- validate DomainStage Correctness articleRubric article
```

The threshold is inclusive: `Correctness` gates both `Correctness` and
`Blocking`, while `Blocking` gates only `Blocking`. `validationFailures` is the
pure first-class projection when a caller wants to inspect the result before
deciding. A failed gate raises `ValidationFailed`; its structured diagnostic
uses code `workflow.validation_failed` and a `validation_failed` array. Each
entry contains `stage` (`structure`, `readability`, `domain`, or `reviewer`),
the Norm identity as `rule`, `expected` data from the Norm, `observed` data
from the Violation, and the Norm's `provenance`. This reuses
`Norm`/`Rubric`/`Critique`; it does not add another checker or upgrade either
`agenstro.norm/v1` or `agenstro.plugin/v1`.

`gateCritique` first validates that the critique classifies every norm in the
supplied rubric exactly once. It rejects foreign norm identities, inconsistent
classifications, and violations with an incorrect severity or invalid locus as
`RequirementFailed`. The pure `validationFailures` projection assumes a critique
from judging that same rubric; use `gateCritique` when enforcing a gate. A
`Critique` does not identify an artifact version, so keep it paired with the
candidate it actually checked and judge again after changing that candidate.

An unchecked norm does not count as a violation, including at `Blocking`.
Both the default refinement policy and these severity gates permit unchecked
norms. When a workflow requires every norm to have been checked, express that
additional evidence requirement explicitly:

```haskell
critique <- judge articleRubric article
complete <- requireBecause "required checks are incomplete"
  (null . critiqueUnchecked) critique
validated <- gateCritique DomainStage Correctness articleRubric complete
```

This completeness requirement is a workflow policy. It does not establish that
the checker is correct or that the artifact meets requirements outside the rubric.

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
