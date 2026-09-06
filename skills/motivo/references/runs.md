# Running and reading a method

Initialize the project with `tactus init PROJECT --sdk PATH_TO_CLEF_SDK`.
New workspaces receive both skills, independent Motivo templates, a report asset
and the `motivo.test` effect registration. Initialization preserves existing
files. Re-running init does not replace a project's modified templates/config.

For older workspaces, if the experiment effect is missing, add only this entry to
`.tactus/tactus.toml`, preserving providers and other project settings:

```toml
[effects."motivo.test"]
command = ["motivo-test"]
```

An absolute installed binary path is valid. Check isolation support with
`tactus plugin-call motivo.test describe --namespace effect --root PROJECT --json`; absence of the plugin does not prevent non-experiment
method reports. Do not run a test without the enforcing plugin.

## Record notes prepared by the current agent

Save notes below `.tactus/motivo/drafts/`, optionally using the headings in
[methods.md](methods.md). Optional `--request FILE` preserves a separate original
request; otherwise `request.md` contains the submitted notes. For example:

```markdown
## Question
Why does the importer scan the same directory twice?
## Sources
src/importer.ts:42-77; trace captured in artifacts/import-trace.txt.
## Findings
The first scan checks names and the second reads content. The trace confirms two traversals.
## Unknowns
Whether the duplicate scan materially affects the reported delay remains unmeasured.
## Next step
Use a small directory fixture to compare one traversal with two.
## Local reflection
- The trace narrowed the issue to duplicate traversal rather than parsing.
- The relative cost of traversal and parsing remains unknown.
- Reading unrelated UI modules is not needed for this question.
```

Run the existing template without changing it:

```sh
tactus run --root PROJECT --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/020_investigate.hs \
  -- --input .tactus/motivo/drafts/investigation.md --agent 'Claude Code'
```

`--agent` and optional `--model` are one-line, caller-reported provenance labels;
omit an unknown value. Without `--provider`, they do not configure or start a provider. With explicit
`--provider`, `--model` is the requested model override; the report does not claim
the provider actually used that model unless observed. `--run-id TOKEN`
sets a deterministic identity; an existing run is rejected rather than replaced.
The default is a fresh timestamp identity. `--parent-run-id TOKEN` links a later
method to earlier work. Run IDs use 1–80 ASCII letters, digits, `_` or `-`.

For an independent investigation, explicitly select a configured provider:

```sh
tactus run --root PROJECT --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/020_investigate.hs \
  -- --input .tactus/motivo/drafts/question.md --provider claude-code
```

Optional `--model MODEL_NAME` is passed through `ProviderRef` as a request. This
starts exactly one separate context with the chosen method's instructions. It
does not inherit the interactive CLI's memory, model, or sandbox; read-only
investigation is a prompt instruction, not OS filesystem enforcement. The raw
response is preserved in `artifacts/provider-response.md`. Missing report fields
remain unknown rather than causing an automatic retry. Probe rejects
`--provider` and always uses only the enforcing experiment effect.

If you actually edit a template/helper, check only that source first:

```sh
tactus check --root PROJECT --scripts-dir .tactus/motivoscript \
  .tactus/motivoscript/020_investigate.hs
```

The templates use only Clef and its existing Haskell dependencies; no additional
Cabal package is required. Business `tactus run --all` remains scoped to
`.tactus/scripts`. Do not use Motivo `--all` to force every method to run.

## Run one local experiment

Choose a fresh run and sample ID, and prepare the necessary fixture under that
sample directory. Do not create the plugin's own reserved log directory. A
command can read project sources through `/project`; use relative paths to work
with writable fixture copies under `/work`.

```sh
mkdir -p PROJECT/.tactus/motivotest/probe-example/sample-001
# Prepare the small fixture or reproduce.py in the directory above.
tactus run --root PROJECT --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/050_probe.hs \
  -- --input .tactus/motivo/drafts/probe.md \
     --run-id probe-example --sample-id sample-001 --timeout-seconds 60 \
     -- python3 reproduce.py
```

The Haskell template passes an explicit argv array to `motivo.test` using
`perform (operation "motivo.test" "run" params)`. It does not start a model. A
shell is used only if explicitly included in the argv, for example
`sh -c 'python3 one.py && python3 two.py'`; both commands share one experiment
budget. Prefer direct argv when a shell is unnecessary.

The sandbox exposes `/project` read-only and `/work` as the only persistent
writable location. It has no network and no host home access. Tool caches,
temporary files and generated output belong under `/work`. The process deadline
is at most 600 seconds, including experiment setup and work; a smaller explicit
value is accepted. Each sample runs once. If the isolation backend or required
executable is unavailable, record that failure rather than retrying outside it.

The effect returns `status` (`exited`, `timed_out`, or `cancelled`), `exit_code`,
`signal`, `duration_ms`, the timeout and log paths, plus log completeness flags.
A normal nonzero exit becomes `experiment-failed` in the report. Timeout becomes
`timed-out`. Missing/unusable terminal results become `needs-check`, and partial
artifacts remain. The original hypothesis is retained alongside the observation.

## Files and reading

Each invocation creates:

```text
.tactus/motivo/
  index.html                         offline overview, regenerated by records
  runs/<run-id>/
    request.md                       supplied request, or original submitted notes
    run.json                         method, state, caller identity, run links
    samples.jsonl                    append order and times of recorded observations
    report.md                        readable method report
    report.html                      standalone offline report
    artifacts/
      submitted-notes.md             unchanged input
      retrospective.md               only for a retrospective
      handoff.md                     extracted retrospective handoff
      decision-history.json          extracted review evidence; no invented dates
      provider-response.md           only after explicit --provider
      experiment.json                only for a terminal experiment result
      experiment-error.json          only for interrupted/failed transport
.tactus/motivotest/<run>/<sample>/    writable experiment fixtures and logs
.tactus/runs/<tactus-run-id>/         runtime-owned invocation diagnostics
```

Tell the user to open `index.html` or `report.html` in a browser. There is no
server, provider selection UI, or execution button. Updates replace complete
HTML files atomically. Refreshing the page reads the newest generated snapshot;
an idle page cannot fetch live agent progress. The page states when it was
generated. The timeline shows at most 200 recent recorded events; `samples.jsonl`
retains the sequence. The overview shows the latest 200 run records.

`recorded` means method notes were published, not that the user's goal was
achieved. `responded` means an explicitly selected provider returned; it does not
prove its claims. `running` refers to an observed experiment at the last snapshot; after
a hard crash it can be stale. Inspect logs/runtime state before deciding what
happened. Runtime events and timestamps provide observations, not the agent's
private reasoning or proof of a causal order across parallel work.

Method entries are atomic units of purpose, not transactions. There is no
rollback of business files, no snapshot restoration, and no automatic replay.
