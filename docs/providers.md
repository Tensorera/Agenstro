---
title: Configure coding-agent providers
status: alpha
owners: [tactus]
last_verified: 2026-09-18
applies_to: "tactus-runtime provider adapters 0.3.0"
platforms: [windows, ubuntu]
---

# Configure coding-agent providers

Tactus includes adapters for Codex CLI, Claude Code, OpenCode, and Google
Antigravity CLI. This guide
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
| `antigravity` | `agy` | `effort` becomes Antigravity `--effort` |

The accepted model and effort/variant strings are provider-specific and may
change with the installed CLI. Tactus deliberately treats them as open strings.

## Verify native commands

Run these in the same terminal or desktop environment that starts Tactus:

```powershell
Get-Command codex,claude,opencode,agy -All
codex --version
claude --version
opencode --version
agy --version
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

[providers.antigravity]
command = ["tactus", "provider-host", "antigravity"]
# Optional model and effort use the installed agy CLI's supported values.
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

## Google Antigravity CLI

The adapter targets Google's `agy` CLI. Its headless stream contract was checked
against the [official documentation](https://antigravity.google/docs/cli/headless/)
and local CLI 1.2.1 help/version. Native account/model calls are not part of the
offline acceptance gate.

New workspaces include `providers.antigravity`. In an existing workspace,
add that provider table to `.tactus/tactus.toml`; `init` and `skills update`
preserve existing provider configuration. If executable lookup is ambiguous,
pin the native path under `[providers.antigravity.options]` with `executable`.
Then select it as a business or independently configured daemon provider:

```sh
tactus smoke provider:antigravity
tactus generate --provider antigravity "Create the requested Haskell workflow."
tactus run --script .tactus/scripts/010_work.hs --daemon --daemon-provider antigravity
```

Each call sends one JSON `user` message through stdin and closes the pipe.
Prompts are absent from argv. Antigravity `agent_response` deltas feed the live
viewer; the final response replaces accumulated text in the returned value.
A complete, correlated `SUCCESS` result is required. Partial text, missing or
malformed terminal records, cancellation and interruption remain unknown.
A native successful response still does not prove that all requested tools or
business effects completed; inspect task evidence.

The adapter disables native self-updating per process with
`AGY_CLI_DISABLE_AUTO_UPDATE=true`. Authenticate through the native CLI before
live work. Offline smoke checks only version; `auth_status=true` is unsupported
because `agy` has no documented noninteractive auth-status command.

Antigravity's own print timer can return partial output with a successful exit,
as documented in its [CLI changelog](https://antigravity.google/changelog).
Tactus therefore places that timer 60 seconds beyond its supervising deadline,
so Tactus records deadline expiry as `OutcomeUnknown` first. Explicit positive
`options.timeout_seconds` controls the host deadline; direct provider-host
requests without one use a 300-second fallback. Ordinary registered calls keep
the workspace-derived deadline. Use this option rather than overriding the
native print timer through `extra_args`; the adapter rejects `--` and
`--print-timeout` overrides there before starting the native process.

## Optional task daemon

Enable the daemon separately for each run:

```sh
tactus run --script .tactus/scripts/010_work.hs --daemon --daemon-provider reviewer
```

The daemon injects periodic prompts through the selected registered provider.
It observes the running entry and reports anomalies. Configuring a daemon
provider does not enable it on ordinary runs. A separate registry entry allows
its model, effort, executable and options to differ from the business provider:

```toml
[providers.reviewer]
command = ["tactus", "provider-host", "claude-code"]
effort = "high"

[daemon]
provider = "reviewer"
interval_seconds = 60
timeout_seconds = 120
max_retries = 1
retry_scripts = [".tactus/scripts/010_work.hs"]
```

`retry_scripts` explicitly authorizes automatic repair and rerun of those
entries after a known failure. Only include scripts whose effects can safely
be repeated. Without an allowlist the daemon monitors and escalates. A command
can add one entry with `--daemon-retry-script PATH`, repeated as needed.
`--daemon-max-retries` and `--daemon-interval-seconds` override the workspace
policy for this run; all daemon override flags require `--daemon`.

When an authorized entry fails, the provider may propose one exact textual
replacement in that entry. Tactus checks the source has not changed, requires
a unique nonempty match and bounds the replacement to 8 KiB per fragment.
It applies the proposal and reruns only that entry with its original arguments.
The default maximum is one repair per entry. Result ambiguity, cancellation,
invalid proposals, out-of-scope changes and an exhausted budget are escalated
instead of retried. The daemon's native provider retains its configured tool
authority; these proposal rules do not sandbox the provider itself.

The daemon provider returns one JSON object, either directly as its plugin
result or as JSON in the built-in adapter's `text` value:

```json
{"action":"continue","summary":"The task is still progressing."}
```

```json
{"action":"escalate","summary":"The failure requires a business decision."}
```

```json
{"action":"repair","summary":"Correct the local name.","old":"misspelledName","new":"correctName"}
```

Monitoring accepts only `continue` or `escalate`; repair is considered after a
known failure. Reports are saved in the task's run directory as `daemon.jsonl`,
and escalations are also printed to stderr. This is local reporting; it does
not send email, chat messages or webhooks. Daemon invocations can contact or bill
the independently configured provider.

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

All four built-ins recognize these option keys while retaining an open option
object for forward compatibility:

```toml
[providers.codex.options]
executable = "tools/codex"
timeout_seconds = 13440
extra_args = ["--provider-specific-flag"]
extra_env = { NAME = "value" }
auth_status = false
command_prefix = []
```

| Option | Meaning |
| --- | --- |
| `executable` | Native CLI path or PATH command; relative paths resolve from the workspace root |
| `timeout_seconds` | Positive native provider deadline; defaults to 60 seconds below Tactus dispatch (13,440 seconds under the default policy) |
| `extra_args` | Appended to the native provider argv |
| `extra_env` | Additional environment variables for the native provider |
| `auth_status` | Ask `smoke` to run the provider's authentication-status command |
| `command_prefix` | Wrapper argv placed before the native executable; mainly for controlled testing or launchers |
| `native_max_line_bytes` | Maximum one-line native event; default 8 MiB, ceiling 32 MiB |
| `native_max_stdout_bytes` | Cumulative native stdout drain; default 1 GiB, ceiling 4 GiB on 64-bit builds |
| `native_max_result_bytes` | Retained terminal result text; default 4 MiB, ceiling 5 MiB |
| `native_max_stderr_bytes` | Retained native stderr; default 1 MiB, ceiling 16 MiB |
| `native_output_queue_bound` | Lines awaiting parsing; default 8, ceiling 128, additionally bounded to 256 MiB resident capacity |

Bare executable names must resolve to exactly one PATH candidate. If several
installations match, `doctor`, `smoke`, and `invoke` report the ambiguity; set
`executable` to one absolute or workspace-relative path to pin the intended
installation. On Windows, resolution follows `PATHEXT` and ignores
extensionless npm POSIX shims when selecting the native launcher.

Built-in provider/effect hosts accept the workspace's 1 MiB request default
and enforce a 16 MiB hard request ceiling. Native stdout and retained result
budgets are separate from the outer plugin-v1 frame budget.

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
tactus smoke provider:antigravity
```

Live smoke sends a small request and can contact or bill the service:

```powershell
tactus smoke provider:opencode --live
```

Run live smoke only after inspecting the selected model, account, working
directory, and native CLI policy.

## Permission behavior

Permissions are an invocation request, independent of model selection and the
open `options` map. Set startup defaults in `.tactus/tactus.toml`:

```toml
[providers.codex.permissions]
allowed_tools = ["Read"]
allowed_skills = []

[providers.codex.permissions.sandbox]
mode = "read-only"
# Optional absolute native working area, using this machine's path syntax:
# workspace = "/srv/isolated/project"
```

Clef `ProviderRef.providerRefPermissions` and provider `invoke` JSON
`params.permissions` can replace that entire object for each call. Omission
inherits the registry default; an explicit empty object requests normal mode.
Within the object, omission means normal behavior and `[]` requests an empty
allowlist. Consecutive and concurrent calls do not rewrite the registry or
native provider configuration files.

For example, two calls can use independently authored permission objects:

```haskell compile
{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

import Clef
import qualified Data.Text.IO as Text

main :: IO ()
main = do
  let reader = (providerRef "codex")
        { providerRefPermissions = Just (emptyPermissions
            { invocationSandbox = Just (SandboxRequest ReadOnly Nothing) }) }
      ordinary = (providerRef "codex")
        { providerRefPermissions = Just emptyPermissions }
      explainTask = textTask "explain" (\topic -> "Explain briefly: " <> topic)
  answer <- runTactus $ do
    first <- invokeWith reader explainTask "the configured workspace"
    invokeWith ordinary explainTask first
  Text.putStrLn answer
```

| Provider | Tools | Skills | Sandbox |
| --- | --- | --- | --- |
| Codex | Unsupported; warn and retain normal tools | Unsupported; warn and retain normal discovery | Native sandbox mode and absolute working directory |
| Claude Code | Partial: native builtin `--tools`; MCP remains governed by native configuration | Unsupported; warn and retain normal discovery | Unsupported; warn and retain normal mode |
| OpenCode / Antigravity | Unsupported; warn and retain normal mode | Unsupported; warn and retain normal mode | Unsupported; warn and retain normal mode |
| Custom provider plugin | Per-field `describe.permissions_capabilities` declaration | Same | Same, including supported modes and custom workspace support |

An unsupported field falls back independently; supported fields still apply.
Malformed requests and native arguments that contradict a supported requested
restriction are rejected before starting the model call. A legacy or invalid
capability declaration receives the ordinary request without unsupported
permission fields. Tactus does not retry the model invocation to obtain a
fallback.

Read `permission_report.requested`, `effective`, `applications` and `warnings`
in the provider result. `provider.permission.warning` events also reach the
journal, Clef dispatch stream and text-mode warning output. Custom capability
queries include each call's permission request. Their
`permission_report_basis = "capability_declaration"` identifies the report as
negotiated configuration sent to the plugin, not execution attestation. Any
plugin-supplied receipt is preserved as `provider_permission_report`. A scalar
custom result retains its original value shape; the report remains available
as a `provider.permissions` event. Durable journals retain application statuses,
warning codes, and value fingerprints; the live result/event carries the full
requested/effective configuration.

Normal mode preserves the previous adapter behavior: Codex and Claude use
native approval bypass flags, Antigravity uses its approval-bypass flag, and
OpenCode receives the existing allow-oriented override. A supported Codex
sandbox request replaces its hardcoded sandbox bypass. These native mappings
do not sandbox the Tactus process, arbitrary Haskell code, wrapper commands,
or arbitrary configured plugins.

Initialization and permissions negotiation do not overwrite users' `.codex`,
`.claude`, `.opencode`, `.agents`, or global instructions. Unsupported skill or
sandbox isolation can still load native user configuration in normal mode;
the warning explicitly prevents treating that fallback as an isolated call.
Native CLI updates can change semantics; tests use fake native executables,
and the authenticated live matrix is not exercised in public CI.

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
