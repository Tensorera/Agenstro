# Configure a Clef Profile

Use a Clef Profile to configure Clef SDK runtime, workspace, and
storage behavior. This file does not configure Motivo Studio.

## Create the file

Create `profile.toml` beside the workspace and state directories:

```toml
name = "local"

[adapter]
executable = "opencode"
agent = "build"
output_format = "json"
pure = false
auto_approve = true
inherit_environment = true
required_env = []
extra_args = []

[runtime]
max_concurrency = 1
max_attempts = 1
retry_backoff_seconds = 0
fail_fast = true
max_subagent_depth = 1
max_fan_out = 8
session_reuse = "never"

[workspace]
root = "./workspace"
read_roots = []

[storage]
state_root = "./state"
cas_dir = "cas"
traces_dir = "traces"
cache_dir = "cache"
manifests_dir = "manifests"
cache_enabled = false
fsync = false

[labels]
environment = "local"
```

Relative paths resolve from the directory containing `profile.toml`.

## Route logical effort tiers

Tasks may select one of four logical tiers with, for example,
`SessionTask(..., effort=Effort.XHIGH)`. Map only the tiers you intend to use:

```toml
[adapter.models]
medium = "provider/model-with-default-variant"

[adapter.models.xhigh]
model = "openai/gpt-5.6-sol"
variant = "low"

[adapter.models.high]
model = "provider/another-model"
```

Clef `xhigh`, `high`, `medium`, and `low` are route names, not the concrete
model's native effort. Thus logical `xhigh` may intentionally use variant
`low`. Omitting `SessionTask.effort` keeps the existing single
`adapter.model` / `adapter.variant` selection. An explicit tier without a
matching route is an error.

Compilation checks selected model IDs with `opencode models`. A successful
catalog lookup rejects missing models immediately; an unavailable catalog is
reported as a compiler warning and checked again at runtime. Runtime never
sends a probe prompt: if the catalog still cannot establish availability, the
TaskRun is rejected as a non-retryable configuration error before
`opencode run`.

## Load and inspect the Profile

```python
from clef_sdk import load_profile

profile = load_profile("profile.toml")
print(profile.name)
print(profile.workspace.root)
```

`load_profile()` returns an immutable `Profile`. Unknown fields and invalid
paths raise a profile validation error.

## Keep permissions in OpenCode

Clef forwards work to OpenCode. Shell, network, edit, delete, and move
permissions belong in OpenCode's native configuration. Clef does not mirror
those permission rules in the Profile.

`adapter.auto_approve = true` adds OpenCode's non-interactive approval option.
An explicit OpenCode `deny` rule still applies.

See [Profile boundaries](../explanation/clef/profile-boundaries.md) for the
validation and dependency-injection model. See
[Clef profiles reference](../reference/clef/profiles.md#profile) for all
Profile fields.
