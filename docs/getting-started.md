---
title: Build the first Agenstro workflow
status: alpha
last_verified: 2026-08-17
applies_to: "Clef Haskell 0.3.0.0 and Tactus Rust 0.3.0"
---

# Build the first Agenstro workflow

This tutorial creates one model-free Clef workflow, checks it, and runs it
through Tactus. It then shows where provider-assisted generation fits without
mixing live model access into the offline proof.

## Before starting

Complete [Installation](install.md) and verify:

```powershell
tactus --version
ghc --numeric-version
cabal --numeric-version
```

Choose a disposable or already trusted project directory. Tactus does not
sandbox Haskell code or plugins.

## 1. Initialize the workspace

From the project root:

```powershell
$repoRoot = (Resolve-Path D:\src\Agenstro).Path
$projectRoot = "D:\work\hello-agenstro"
New-Item -ItemType Directory -Force $projectRoot | Out-Null
Set-Location $projectRoot

tactus init --sdk (Join-Path $repoRoot "clef-sdk")
tactus doctor
```

The important result is:

```text
.tactus/
  tactus.toml       provider, effect, and plugin registry
  cabal.project     link to the Clef package
  PROMPT.md         generation instructions
  scripts/          Haskell entries and helper modules
  runs/             diagnostic event journals and summaries
  skills/tactus/    agent guidance for editing workflows
```

`init` preserves existing files. Use [Tactus workspace and configuration](tactus-workspace.md)
before manually changing this layout.

## 2. Add an offline Clef program

Create `.tactus/scripts/010_offline.hs`:

```haskell
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

This is an ordinary Haskell executable. `Workflow` supplies typed composition;
`requireBecause` stops the workflow if the stated condition is false.

## 3. Discover and check it

```powershell
tactus list
tactus check .tactus\scripts\010_offline.hs
```

`list` distinguishes runnable numbered entries from helper modules. `check`
asks Cabal and GHC to compile without running workflow code.

The first check on a new machine is a cold build. Cabal may download package
metadata and compile dependencies. A longer finite budget can be supplied:

```powershell
tactus check --timeout-seconds 7200 .tactus\scripts\010_offline.hs
```

## 4. Run it

```powershell
tactus run --script .tactus\scripts\010_offline.hs
```

The final program output is `42`. Tactus also creates a run record below
`.tactus/runs`. Human-facing Agenstro messages use only `[state]`, `[info]`,
`[warning]`, and `[error]`; structured diagnostic evidence stays in the run
journal. See [Logs and run evidence](observability.md).

## 5. Add more stages

Runnable files use increasing three-digit prefixes:

```text
.tactus/scripts/
  010_discover.hs
  020_transform.hs
  030_review.hs
  Tactus/Shared.hs
```

With no explicit selection, Tactus checks all Haskell sources and runs
numbered entries in numeric/path order:

```powershell
tactus check
tactus run
```

To run only later entries, repeat `--script` in the desired order:

```powershell
tactus run `
  --script .tactus\scripts\020_transform.hs `
  --script .tactus\scripts\030_review.hs
```

Provider choice is inside the Haskell workflow or the workspace default. Script
selection does not change the provider.

## 6. Generate a workflow with a provider

First complete [Provider setup](providers.md). Then ask one configured provider
to create or extend scripts:

```powershell
tactus generate --provider codex `
  "Create a typed three-stage workflow: discover inputs, transform them, then review the result."
```

Generation reads `.tactus/PROMPT.md` and the bundled Tactus skill. It may create
new numbered entries or update existing scripts, but it does not automatically
run them. Always inspect and check the result:

```powershell
tactus list
git diff -- .tactus\scripts
tactus check
```

A later `generate` call sees the current workspace and may add or modify
scripts. It does not erase earlier workflows by policy, so state the desired
scope precisely and use version control.

## 7. Read the next guide

- Learn the EDSL: [Program with Clef](clef.md).
- Understand files and defaults: [Tactus workspace and configuration](tactus-workspace.md).
- Add external capabilities: [Author a local plugin](plugin-authoring.md).
- Keep a task alive between runs: [Segno persistent tasks](segno.md).
- Open the workspace visually: [Motivo Studio](motivo-studio.md).
