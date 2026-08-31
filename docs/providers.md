---
title: Configure coding-agent providers
status: alpha
owners: [tactus]
last_verified: 2026-08-17
applies_to: "tactus-runtime provider adapters 0.3.0"
platforms: [windows, ubuntu]
---

# Configure coding-agent providers

Tactus includes adapters for Codex CLI, Claude Code, and OpenCode. This guide
explains executable discovery, authentication ownership, model and reasoning
settings, permission behavior, and safe smoke testing.

## Responsibility boundary

Tactus does not create a provider account, store a token, choose a model
catalogue, or bypass an organization policy. Install and authenticate each
native CLI using that provider's own supported mechanism.

| Tactus registry name | Native executable | Reasoning setting |
| --- | --- | --- |
| `codex` | `codex` | `effort` becomes Codex `model_reasoning_effort` |
| `claude-code` | `claude` | `effort` becomes Claude `--effort` |
| `opencode` | `opencode` | `effort` becomes OpenCode `--variant` |

The accepted model and effort/variant strings are provider-specific and may
change with the installed CLI. Tactus deliberately treats them as open strings.

## Verify native commands

Run these in the same terminal or desktop environment that starts Tactus:

```powershell
Get-Command codex,claude,opencode -All
codex --version
claude --version
opencode --version
```

Only the configured provider must exist. On Windows, Tactus resolves native
`.exe`, `.cmd`, and other executable extensions rather than passing an npm
launcher file directly to `CreateProcess`.

`tactus doctor` checks registry command availability. The built-in provider
host then finds the native provider executable when the adapter runs.

## Configure the default provider

Edit `.tactus/tactus.toml`:

```toml
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus", "provider-host", "codex"]
model = "provider-specific-model"
effort = "high"

[providers."claude-code"]
command = ["tactus", "provider-host", "claude-code"]
model = "provider-specific-model"
effort = "high"

[providers.opencode]
command = ["tactus", "provider-host", "opencode"]
model = "provider/model"
effort = "high"
```

`default_provider` affects:

- Clef `invoke` calls that do not use `invokeWith`; and
- `tactus generate` when `--provider` is omitted.

An explicit CLI or Clef override wins for that call:

```powershell
tactus generate --provider opencode "Add a review stage to the existing workflow."
```

```haskell
invokeWith
  ( (providerRef "opencode")
      { providerRefModel = Just "provider/model",
        providerRefEffort = Just "high"
      }
  )
  task
  input
```

## OpenCode model and effort

OpenCode can expose many configured models. Put the exact OpenCode model ID in
`model`. Tactus passes it as `--model`. Put the desired OpenCode variant in
`effort`; the adapter passes it as `--variant`:

```toml
[providers.opencode]
command = ["tactus", "provider-host", "opencode"]
model = "provider/model-id"
effort = "high"
```

The more explicit adapter spelling is also accepted inside open options and
takes precedence over the generic `effort` value:

```toml
[providers.opencode.options]
variant = "high"
```

Use one style consistently. The top-level `effort` form is easier to share
with Clef's `ProviderRef`; `options.variant` makes the OpenCode mapping
explicit.

## Adapter options

All three built-ins recognize these option keys while retaining an open option
object for forward compatibility:

```toml
[providers.codex.options]
timeout_seconds = 1800
extra_args = ["--provider-specific-flag"]
extra_env = { NAME = "value" }
auth_status = false
command_prefix = []
```

| Option | Meaning |
| --- | --- |
| `timeout_seconds` | Positive native provider deadline; separate from the outer Tactus command deadline |
| `extra_args` | Appended to the native provider argv |
| `extra_env` | Additional environment variables for the native provider |
| `auth_status` | Ask `smoke` to run the provider's authentication-status command |
| `command_prefix` | Wrapper argv placed before the native executable; mainly for controlled testing or launchers |

Avoid putting credentials in `tactus.toml`. Use the native CLI's credential
store or a minimal session environment. Configuration, prompts, errors, paths,
and model output can all be sensitive even when journal fields are summarized.

## Smoke tests

Offline smoke checks executable/version behavior and does not send a model
prompt:

```powershell
tactus smoke provider:codex
tactus smoke provider:claude-code
tactus smoke provider:opencode
```

Live smoke sends a small request and can contact or bill the service:

```powershell
tactus smoke provider:opencode --live
```

Run live smoke only after inspecting the selected model, account, working
directory, and native CLI policy.

## Permission behavior

The reference adapters are intended for an already trusted local workflow:

- Codex is invoked with its dangerous approval/sandbox bypass;
- Claude Code is invoked with its dangerous permission bypass; and
- OpenCode receives an allow-oriented permission override, but Tactus reports
  `full_bypass = false` because an explicit deny or managed policy can still
  win.

These flags do not make Agenstro a sandbox and do not override operating-system
permissions. Native CLI updates can change semantics; the authenticated live
matrix is not exercised in public CI.

## Generation versus workflow execution

`tactus generate` uses one provider to create or revise Haskell source. It does
not run those scripts. A later `tactus run` executes the providers named by the
workflow itself or, for `invoke`, the current `default_provider`.

Therefore this:

```powershell
tactus generate --provider claude-code "Add two new workflow stages."
```

does not permanently select Claude for all generated tasks. Provider selection
inside those Haskell scripts and `default_provider` still control execution.

## Diagnose failures

Use this order:

```powershell
tactus doctor
tactus smoke provider:NAME
tactus smoke provider:NAME --live
```

An executable failure is a known setup failure. A timeout, broken protocol, or
lost terminal after an external request may become `OutcomeUnknown`; inspect
the provider account and workspace before retrying. See [Troubleshooting](troubleshooting.md)
and [Logs and run evidence](observability.md).
