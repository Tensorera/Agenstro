---
name: tactus
description: Write, check, and run Tactus Haskell workflows using Clef, and inspect workflow outcomes and live output. Provider model calls must originate in a Clef Haskell workflow. Use for .tactus/scripts, workspace configuration, typed provider calls, effect plugins, or diagnosing running workflows. Persistent scheduling belongs to Segno; task-solving methods belong to Motivo or the caller.
---

# Tactus Haskell workflows

Use the workspace's installed `tactus` CLI and Clef SDK. For authoring, start
with `tactus list --root <workspace> --json`; inspect the selected source and
needed imports. For supervision and live output, use the corresponding
reference below. Use `tactus doctor --root <workspace> --json` to diagnose setup
errors.

## Provider calling rules

- Call a model or provider only from a Haskell `Workflow` through Clef's
  `invoke` or `invokeWith`. Write the task in Haskell, check its source, then
  execute the selected entry with `tactus run`.
- Refuse provider invocation commands implemented in shell, Python,
  JavaScript, TypeScript, or another language. Do not run native `codex`,
  `claude`, `opencode`, or `agy` commands, `tactus provider-host`, provider
  `plugin-call`, `tactus generate`, or `tactus smoke --live` as an alternative
  provider calling channel. Do not construct raw dispatch requests or use
  Haskell `System.Process` to bypass Clef's provider APIs.
- Shell commands may initialize, list, check, run, or inspect a Haskell
  workflow. That does not authorize a shell script to make provider calls
  directly. When another calling channel is requested, keep the provider task
  in a Clef Haskell entry instead.
- Daemon monitoring and repair call providers directly from the Rust runtime.
  They require separate, explicit user authorization and are outside this
  skill's default provider calling channel. An ordinary workflow request does
  not authorize daemon monitoring or repair.

These are this skill's calling rules; they do not provide an operating-system
sandbox. Skills are installed under `.tactus/skills` for explicit reading;
initialization does not register them with the user's coding-agent host.

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

## Per-call permissions

Set `providerRefPermissions` on the `ProviderRef` passed to `invokeWith` when
one call needs its own permission request. `emptyPermissions` leaves
`allowedTools`, `allowedSkills`, and `invocationSandbox` unspecified;
`Just []` for a tool or skill field requests an empty selection. An omitted
`providerRefPermissions` inherits startup permissions; `Just emptyPermissions`
explicitly restores the provider's ordinary invocation behavior.

For example, this Codex call requests its supported read-only sandbox:

```haskell compile
{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

import Clef
import qualified Data.Text.IO as Text

main :: IO ()
main = do
  let selected = (providerRef "codex")
        { providerRefPermissions = Just (emptyPermissions
            { invocationSandbox = Just (SandboxRequest ReadOnly Nothing) })
        }
  result <- runTactus $ invokeWith selected
    (textTask "explain" (\topic -> "Explain briefly: " <> topic))
    "the difference between parsing and validation"
  Text.putStrLn result
```

Support depends on the adapter and field. Read the recorded requested and
effective permission report and its warnings before describing a restriction
as applied. An unsupported field falls back to the provider's ordinary
behavior with a warning; it does not establish that requested tool, skill, or
sandbox isolation. Keep provider calls in the Haskell workflow even when a
field is unsupported.

Read [references/commands.md](references/commands.md) for selection and plugin
syntax. For periodic monitoring, bounded automatic repair, or the live output
viewer, read [references/supervision.md](references/supervision.md). Its daemon
section explains the separately authorized runtime feature; reading it does
not authorize provider monitoring or repair.

For persistent or scheduled tasks, use the sibling
[Segno skill](../segno/SKILL.md) to author the timestamp organizer, register the
workspace, and manage durable invocation state. Keep business scripts and their
working-directory arguments in Tactus. For task-solving methods and evidence
reports, use the sibling [Motivo skill](../motivo/SKILL.md).

Never blindly retry `OutcomeUnknown`, a timeout after dispatch, or a
missing terminal response: inspect the recorded and external outcome first.
See [references/outcomes.md](references/outcomes.md) for diagnostic commands.
