---
title: Start with your coding agent
status: alpha
owners: [documentation]
last_verified: 2026-09-05
applies_to: "Clef Haskell 0.3.0.0 and Tactus Rust 0.3.0"
platforms: [windows, ubuntu]
---

# Start with your coding agent

Use Agenstro in three steps: initialize a folder, open your chosen coding-agent
CLI there, and ask that agent to help complete the work. Motivo supplies methods
for uncertainty and coordination. Clef and Tactus execute explicit Haskell
workflows when the work calls for them.

The commands below also serve as a small offline acceptance exercise; the agent
can perform them for you. You do not need to start a desktop client, configure
a second conversation, or learn Haskell before describing the task.

## Before starting

Complete [Installation](install.md), then verify the installed tools:

```text
tactus --version
ghc --numeric-version
cabal --numeric-version
```

Use a disposable or trusted project for the first exercise. Ordinary Tactus
Haskell and plugins can perform `IO`; the specific `motivo.test` effect provides
the separate Linux experiment sandbox.

## 1. Initialize the project folder

From a terminal, point Tactus to the project and the installed Clef package.
On Linux, for example:

```sh
tactus init /path/to/project --sdk /path/to/Agenstro/clef-sdk
cd /path/to/project
```

The corresponding Windows command uses Windows paths:

```powershell
tactus init D:\work\project --sdk D:\src\Agenstro\clef-sdk
Set-Location D:\work\project
```

The relevant workspace layout is:

```text
.tactus/
  tactus.toml       provider, effect and plugin registry
  cabal.project     link to the Clef package
  PROMPT.md         instructions for workflow generation
  scripts/          business Haskell entries and helpers
  motivoscript/     eight independent Motivo entries and shared helpers
  motivotest/       bounded experiment fixtures and results
  motivo/           method reports, samples and index.html
  runs/             Tactus diagnostic journals and summaries
  skills/tactus/    Haskell authoring and runtime rules
  skills/motivo/    method guidance and the offline HTML template
```

`init` preserves existing files and installs short host-agent skill pointers.
For setup failures, run `tactus doctor`; see
[Tactus workspace and configuration](tactus-workspace.md) before modifying the
layout or changing SDK references.

## 2. Open your chosen coding agent here

For example, start either `codex` or `claude` from the initialized project
folder. Keep using that CLI's model, authentication, permission and conversation
controls. Agenstro does not infer those settings from Tactus's default provider.

The main agent stays responsible for understanding your goal, choosing a useful
next action and explaining what changed. No Motivo task service takes over the
conversation.

## 3. Ask the agent to use Motivo when it helps

An initial request can be ordinary language:

> Use Motivo to investigate why importing a large folder is slow. Start with the
> relevant code and existing evidence, record what remains uncertain, and show
> me the report before proposing a focused experiment.

The methods are clarify, investigate, analyze, research, probe, organize,
retrospect and handoff. The agent selects one where it helps; it need not run
all eight or write a custom workflow for a simple task.

By default, a template records the main agent's existing Markdown. It accepts
ordinary prose; missing suggested sections remain unknown rather than requiring
a model call to repair a report format. For example, the agent can save its
notes to `.tactus/motivo/drafts/investigation.md` and execute:

```sh
tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/020_investigate.hs \
  -- --input .tactus/motivo/drafts/investigation.md --agent Codex
```

The output gives the run's `report.html` and `.tactus/motivo/index.html`. Open
one in a browser. Each HTML file contains its own data and needs no server or
port. Refreshing it reads the newest generated snapshot; the page cannot observe
unrecorded agent activity or initiate execution. The agent should still explain
findings and next actions in your conversation.

A non-probe method can request one independent context using explicit
`--provider NAME` and optional `--model MODEL`. This does not inherit the main
agent's memory or silently use a default provider. Probe instead calls the
`motivo.test` effect: fixtures and persistent writes stay below
`.tactus/motivotest/<run>/<sample>`, and the maximum experiment deadline is 600
seconds. If enforcing isolation is unavailable, it refuses the experiment.
See [Motivo Studio](motivo-studio.md) for the complete method and report flow.

## Try one offline business workflow

When a repeatable action needs a Haskell entry, the main agent follows the
Tactus skill and writes it under `.tactus/scripts`. This small example makes no
model call. Save it as `.tactus/scripts/010_offline.hs`:

```haskell
{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

import Clef

main :: IO ()
main = do
  result <- runTactus $ do
    value <- pure (20 :: Int)
    requireBecause "arithmetic invariant" (value + 22 == 42)
    pure (value + 22)
  print result
```

Check and execute only this entry:

```text
tactus list
tactus check .tactus/scripts/010_offline.hs
tactus run --script .tactus/scripts/010_offline.hs
```

The program prints `42`. `check` asks Cabal and GHC to compile without executing
the workflow; a cold build may download and compile dependencies. `run` also
creates diagnostic evidence below `.tactus/runs`. Query it with:

```text
tactus runs summarize --since 24h
tactus runs unfinished
```

Ordinary `list`, `check --all` and `run --all` select only `.tactus/scripts`.
They do not include Motivo methods. A source path, explicit `--all`, or numeric
range is required for check/run; an omitted selection cannot silently execute
the workspace. A logical entry is not a transaction or a rollback mechanism.

Provider-assisted `tactus generate --provider NAME GOAL...` remains available
when explicitly useful. Generation reads `.tactus/PROMPT.md` and the Tactus skill,
may compile-check changes, and does not automatically execute the generated
business workflow. An agent already working in the project can usually author
that entry directly. Keep generation guidance separate from optional shared
business-call `runtime_instructions`.

## Continue from the result

A published method, returned provider response or passing small experiment does
not establish that your whole task is complete. The main agent should connect
the observed evidence to your goal, make the next authorized change, and review
what the actual result teaches.

- Method examples and reports: [Motivo Studio](motivo-studio.md).
- Haskell composition: [Program with Clef](clef.md).
- Source selection and commands: [CLI reference](reference/cli-v0.3.md).
- External capabilities: [Author a local plugin](plugin-authoring.md).
- Persistent execution: [Segno persistent tasks](segno.md).
- Runtime evidence: [Logs and run evidence](observability.md).
