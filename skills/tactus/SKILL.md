---
name: tactus
description: Write, check, and run Tactus Haskell workflows using Clef; configure optional daemon supervision, bounded repair and rerun, and live provider output viewing. Use for .tactus/scripts, workspace configuration, typed provider calls, effect plugins, or diagnosing running workflows. Persistent scheduling belongs to Segno; task-solving methods belong to Motivo or the caller.
---

# Tactus workflows and supervision

Use the workspace's installed `tactus` CLI and Clef SDK. For authoring, start
with `tactus list --root <workspace> --json`; inspect the selected source and
needed imports. For supervision and live output, use the corresponding
reference below. Use `tactus doctor --root <workspace> --json` to diagnose setup
errors.

## Source rules

- Store business Haskell sources below `.tactus/scripts`. Other source trees
  below `.tactus`, such as `.tactus/motivoscript`, require an explicit
  `--scripts-dir` on `list`, `check`, and `run`. Files named
  `NNN_name.hs` or `NNN_name.lhs` are runnable entries; helpers are ordinary Haskell modules.
  Keep selected paths within the selected directory; linked files and directories
  are not accepted as explicit sources. Source selection is not an IO sandbox.
  The standard `.tactus/segno.hs` organizer is an explicit-only exception:
  select it with `--script .tactus/segno.hs`; ordinary `run --all` excludes it.
- Entries declare `module Main (main) where`, import `Clef`, and expose
  `main :: IO ()`. Run a `Workflow a` with `runTactus`; use ordinary Haskell
  functions and values for local computation.
- `Task input output` describes a provider call. `textTask` accepts text;
  `jsonTask` needs a `FromJSON output` instance. `invoke` uses the configured
  default provider; `invokeWith (providerRef "name")` selects one explicitly.
  Decoding checks output structure, not the truth of its claims.
- `operation "effect-name" "method" params` describes a registered effect;
  `perform` executes it. Inputs need `ToJSON`, outputs need `FromJSON`.
  Register project-specific plugins in `.tactus/tactus.toml` when needed.
  Follow the existing `agenstro.plugin/v1` protocol; keep logs off stdout.
- `parallel`, `parallelAll`, and `parallelAllBounded` isolate calls, not their
  filesystem effects. Concurrent branches share the workspace unless the
  caller supplies separate locations. They do not provide transactional rollback.
- `.tactus/PROMPT.md` (`instructions` in `.tactus/tactus.toml`) guides script generation only.
  Shared business-call instructions use the separate optional
  `runtime_instructions` file setting. Existing workspaces may need to move
  business instructions to that file. `tactus init` preserves existing files.

A complete entry, without extra stage or report types:

```haskell
{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

import Clef
import qualified Data.Text.IO as Text

main :: IO ()
main = do
  result <- runTactus $ invoke
    (textTask "explain" (\topic -> "Explain briefly: " <> topic))
    "the difference between parsing and validation"
  Text.putStrLn result
```

Compile the selected source to catch Haskell errors:

```sh
tactus check --root /path/to/project .tactus/scripts/010_explain.hs
```

Execution is a separate command and can call models or modify project files:

```sh
tactus run --root /path/to/project --script .tactus/scripts/010_explain.hs
```

Read [references/commands.md](references/commands.md) for selection and plugin
syntax. For periodic monitoring, bounded automatic repair, or the live output
viewer, read [references/supervision.md](references/supervision.md). Daemon is
opt-in per run and uses provider calls; enable repair only for explicitly
retryable entries whose effects can safely repeat.

For persistent or scheduled tasks, use the sibling
[Segno skill](../segno/SKILL.md) to author the timestamp organizer, register the
workspace, and manage durable invocation state. Keep business scripts and their
working-directory arguments in Tactus. For task-solving methods and evidence
reports, use the sibling [Motivo skill](../motivo/SKILL.md).

Never blindly retry `OutcomeUnknown`, a timeout after dispatch, or a
missing terminal response: inspect the recorded and external outcome first.
See [references/outcomes.md](references/outcomes.md) for diagnostic commands.
