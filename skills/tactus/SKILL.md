---
name: tactus
description: Safely inspect, diagnose, edit, type-check, and run selected Tactus Haskell workflow scripts. Use when a request mentions Tactus, a `.tactus` workspace, files below `.tactus/scripts`, `tactus list/doctor/check/run`, Motivo run evidence, or a Segno `OutcomeUnknown` result.
---

# Tactus

Work through the installed `tactus` CLI and the workspace's ordinary files. Let Tactus own discovery, ordering, configuration validation, execution, and run evidence; independently constrain every agent-edited script path to `.tactus/scripts`.

## Workflow

1. Resolve the requested start path to an absolute path. Search it and its parents for `.tactus/tactus.toml`; do not initialize a workspace unless the user explicitly asks.
2. Run `tactus list --root <path> --json` to confirm the workspace and inventory scripts. Run `tactus doctor --root <path> --json` when environment, SDK, configuration, or plugin health matters.
3. Canonicalize every requested script and require it to remain below that workspace's `.tactus/scripts` directory. Stop if a symlink, junction, `..`, or absolute path escapes it.
4. Read the specified scripts plus only the directly needed local helper modules. Preserve unrelated files and existing user changes.
5. Make the narrow requested edit. Review the diff before invoking Tactus.
6. Type-check only the selected sources when practical. Run only explicitly selected numbered entry points and only when execution is requested or is a clearly required verification step.
7. Report the selected files, exact validation performed, exit status, and concise diagnostics. Keep raw structured details available but do not make raw JSON the default presentation.

Read [references/commands.md](references/commands.md) for exact selection syntax and [references/outcomes.md](references/outcomes.md) whenever a run is ambiguous, timed out, interrupted, or reports `OutcomeUnknown`.

## Authority and safety

- Treat `.tactus/tactus.toml`, `.tactus/runs`, temporary Clef runtime configuration, and Segno lifecycle databases as runtime-owned. Do not edit them unless the user explicitly requests the specific file and understands the boundary.
- Never run every entry merely to validate one edited script. `check` compiles; `run` executes trusted Haskell `IO` and may invoke providers, plugins, processes, or filesystem effects.
- Inspect a selected entry and its imports before running it. Use live providers, credentials, or broad effects only when the requested task actually calls for them; do not infer such execution from a read-only diagnosis.
- Do not use `--keep-going` by default. Preserve the first failure and diagnose it before widening scope.
- Do not treat trace evidence as deterministic replay state or proof that an external effect did not occur.
- Never blindly retry `OutcomeUnknown`, a timeout after dispatch, a lost terminal response, or another ambiguous external result.

## User-facing diagnostics

Summarize output with only these labels:

- `[state]` lifecycle transitions and terminal outcomes
- `[info]` normal progress and factual observations
- `[warning]` partial, ambiguous, degraded, or retry-sensitive conditions
- `[error]` definite failures

Prefer the runtime's canonical presentation category and message when present. Put command lines, stable codes, file/line locations, exit codes, and bounded raw JSON under a clearly marked technical-details section. Do not infer a successful external outcome from an exit code alone.
