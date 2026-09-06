---
name: motivo
description: Help a coding agent clarify, investigate, analyze, research, organize, review, or hand off a complex task, and record bounded local experiments and readable evidence reports. Use when the user asks for Motivo or when uncertainty, coordination, or a failed approach warrants a focused method.
---

# Motivo in your current coding-agent session

You remain the user's first interface. Use the current session to understand the
problem, inspect evidence, choose a useful method, and explain the next action.
Motivo supplies small Haskell templates and observation files. It does not choose
a model, start a second task controller, or require every task to follow a cycle.
Handle work directly when no method would reduce uncertainty or coordination cost.

1. Check the initialized workspace and choose **one** relevant method from
   [references/methods.md](references/methods.md). Prepare a concise Markdown note
   using that method's suggested headings and local reflection questions where
   useful. Missing fields do not prevent recording. State missing
   information as unknown; recording a claim does not verify it.
2. Run the selected template from `.tactus/motivoscript`, following
   [references/runs.md](references/runs.md). By default the templates consume your notes without invoking a model. Use
   explicit `--provider CONFIGURED_NAME` only when a separate context is useful. Prefer filling their input over editing Haskell.
   Business workflows remain under `.tactus/scripts` and follow the Tactus skill.
3. Give the user the generated `.tactus/motivo/index.html` or run's `report.html`
   path and tell them to open it in a browser. It is an offline snapshot: refresh
   reads a newer generated file; it cannot observe unrecorded agent work or run
   commands. Explain your current finding and the smallest useful next step in
   the agent conversation, then continue the user's authorized work.

Methods are independent, not mandatory stages. Do not generate a plan merely to
satisfy a template. Preserve sources, counterevidence, uncertainty and failed
experiments when relevant. Revisit the method only when new evidence changes what
to do. Do not add actors, nested task loops, or reusable plugins without a concrete
repeated need.

## Minimal experiments

Use `050_probe.hs` and the registered `motivo.test` effect for a local experiment.
Prepare only the necessary fixtures below
`.tactus/motivotest/<run-id>/<sample-id>/`. The effect runs the command with a
read-only project at `/project` and that sample directory at `/work`; cwd is
`/work`. It allows at most 600 seconds, or a shorter explicit timeout. The process
cannot write business sources or the host home, and networking is disabled.

Do not run experiments through your ordinary shell or `liftIO` to bypass these
limits. If isolation is unavailable, record the limitation and continue work
that does not need the experiment. Do not downgrade to a working-directory or
prompt-only restriction. Do not split one timed-out experiment into repeated
runs to evade its budget. An intentional business change belongs in a separate,
explicit business workflow, not in an experiment sandbox.

Use a new run/sample identity after inspecting an interrupted attempt. Preserve
its observations. A timeout, missing response, or partial result does not prove
nothing happened, and does not authorize an automatic retry.

## Agent identity and optional separate calls

Keep the user's chosen CLI and model configuration. Report agent/model identity
only when known; `--agent` and `--model` are caller-reported labels, not verified
runtime facts. Never infer the current model from Tactus's default provider.

A separate provider call is optional and must have a concrete reason. Non-probe
templates accept `--provider CONFIGURED_NAME` and optional `--model NAME`; they
use `invokeWith` once and preserve the response without format-repair retries.
Record the selected provider and requested model, and distinguish those requests
from any model identity actually reported. If editing a template to add a call,
continue using explicit `invokeWith (providerRef "configured-name")`. Never use plain `invoke` to silently
switch from the user's active agent to the default provider. Independent calls
have separate contexts; their filesystem effects are not automatically isolated.
