---
title: Motivo methods and offline reports
status: alpha
owners: [motivo]
last_verified: 2026-10-04
applies_to: "Motivo, Clef and Tactus 0.3"
platforms: [windows, ubuntu]
---

# Motivo methods and offline reports

The user's first interface is their existing coding agent. Initialize a folder,
start Claude, Codex or another supported agent there, and ask it to use Motivo.
The agent chooses a method, supplies the question and evidence, and continues
work after reading its result. Motivo Studio is the resulting offline HTML
observation page; it does not start or control agents.

## Start from your coding agent

```sh
tactus init --sdk /path/to/Agenstro/clef-sdk
```

Open your coding agent in the initialized project and ask, for example:

> Use Motivo to investigate why importing a large file stalls. Keep the work
> focused and show me the evidence before deciding what to change.

Initialization installs canonical instructions in `.tactus/skills/motivo`.
Ask the existing coding agent to read `.tactus/skills/motivo/SKILL.md` explicitly;
init does not create host-specific discovery pointers or load these instructions
into provider calls. Existing user instructions, skills and historical discovery
files are preserved.

The agent can point you at `.tactus/motivo/index.html`. Open that file in your
browser. Keep task instructions, model choices and feedback in the original
agent conversation. Opening the report is optional and closing it does not stop
work.

## Eight independent methods

| Method | Question it helps answer | Main material |
| --- | --- | --- |
| clarify | What outcome and constraints are actually intended? | Goal, assumptions, unknowns |
| investigate | What evidence resolves this specific uncertainty? | Findings and source locations |
| analyze | Which explanation fits the available evidence? | Hypotheses, counterevidence, decision |
| research | What external knowledge applies to this decision? | Sources, comparison, limits |
| probe | What small experiment can distinguish the hypotheses? | Prediction, actual observation, logs |
| organize | Which coherent action should happen next? | Work units and real dependencies |
| retrospect | Why did the work change direction, and what should change next time? | Expectations versus observations and decision history |
| handoff | What does the next agent need to continue without repeating work? | Current state, checks, open issues, next step |

These are choices, not an eight-stage pipeline. Every method includes a small
local reflection; it does not invoke an additional reviewer. Simple reasoning
can remain in the main agent. Stable templates consume new input rather than
requiring new Haskell scaffolding for every question.

## Script and data boundaries

```text
.tactus/scripts/        business Haskell entries
.tactus/motivoscript/   method Haskell entries and shared Motivo modules
.tactus/motivotest/     bounded experiment files, grouped by run/sample
.tactus/motivo/         method records, artifacts, index.html
.tactus/runs/           Tactus diagnostic records (unchanged)
```

Tactus `list`, `check`, and `run` accept `--scripts-dir .tactus/motivoscript`.
Omitting it keeps the existing business-script selection. In particular,
`tactus run --all` does not execute installed method templates.

For an agent preparing a retrospective:

```sh
tactus check --scripts-dir .tactus/motivoscript \
  .tactus/motivoscript/070_retrospect.hs

tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/070_retrospect.hs \
  -- --input retrospective-notes.md --agent "current coding agent"
```

The input is ordinary Markdown. The method reference suggests headings and
reflection questions; do not fabricate evidence to fill a section. The default
mode records the main agent's supplied material without contacting a provider.
Where an independent investigation is useful, select an existing Tactus provider
explicitly using the method's `--provider` option. This starts a fresh provider
invocation, not a continuation of the main CLI session. Use the installed
method's `--help` and skill references for the exact options.

A script is atomic as a work unit, not a database transaction. Failure does not
undo its earlier external effects. Parallel calls do not provide isolated
write access to a shared repository.

## Minimal experiments

Install the separate `motivo-test` executable as described in [Installation](install.md).
New workspaces register it as the `motivo.test` effect. Existing `tactus.toml`
files are preserved; add the following entry only if it is absent:

```toml
[effects."motivo.test"]
command = ["motivo-test"]
```

The probe template calls this effect through Clef and Tactus. Its Linux backend
uses Bubblewrap to expose `/project` as the read-only workspace and `/work` as
the sample directory. Experiment programs run in `/work`. Prepare fixtures in
`.tactus/motivotest/<run-id>/<sample-id>`; use relative paths for outputs and
`/project/...` for project inputs. The environment does not inherit the parent
agent's credentials or control sockets.

The default and maximum experiment timeout are 600 seconds; an explicit value
must be an integer from 1 through 600. Preparation, payload compilation, and
execution share one monotonic deadline. The experiment namespace is terminated
at the deadline; saving the final observation is not extra experiment time.
Framework installation and Haskell template preparation are separate from this
bounded experiment and must not be presented as experiment execution.

The current strict backend is Linux only and requires usable Bubblewrap user
namespaces. Unsupported or unavailable isolation refuses execution. A working
directory or a prompt saying “do not edit” is not an equivalent fallback.
Other methods and offline reports remain available without this backend.

A nonzero exit, timeout, or missing terminal result is evidence to inspect.
Motivo does not retry an experiment automatically or promote experimental files
into business source. The main agent chooses any subsequent business Haskell
step using the Tactus skill.

## Executable acceptance and test-driven development

`090_test.hs` and the installed `Motivo.Test` module add command acceptance
suites alongside the eight independent methods. A suite contains named cases;
each case runs one explicit argv command through the same `motivo.test` effect.
The host checks declared assertions against protected stdout/stderr, exit facts
and files under that case's `/work`. It does not accept a provider's `testsRun`
claim as a test result or call a provider implicitly.

Write the acceptance oracle before implementing the behavior. For example:

```json
{
  "api": "motivo.test-suite/v1",
  "id": "addition",
  "cases": [{
    "id": "two-plus-two",
    "command": {
      "argv": ["/usr/bin/python3", "/project/add.py", "2", "2"],
      "timeout_seconds": 15
    },
    "assertions": [
      {"kind": "exit_code", "expected": 0},
      {"kind": "stdout_equals", "expected": "4\n"}
    ]
  }]
}
```

Save it as `acceptance.json`, run red, make the authorized implementation change
outside the experiment sandbox, and run green:

```sh
tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/090_test.hs \
  -- --suite acceptance.json --run-id addition-red --phase red

# Edit add.py as an explicit implementation step.

tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/090_test.hs \
  -- --suite acceptance.json --run-id addition-green \
  --phase green --baseline addition-red
```

Red is established only by at least one actual assertion failure with no
execution or evidence errors. Its exit code remains 1, preserving the fact that
the tests failed. All-pass red, invalid baseline, changed oracle, isolation
failure, timeout, cancellation, signal termination, malformed facts, absent or
truncated logs are errors (exit 2). Normal/green all-pass returns 0; actual failed
assertions return 1. No phase retries a case or edits an implementation.

Green validates the baseline before executing any case. Suite identity, ordered
case ids, argv, timeout values and assertions must match the red run's normalized
manifest exactly. Implementation contents can change. The saved baseline is
local caller-controlled evidence, not a signature or a defense against someone
rewriting the reports themselves.

The manifest does not freeze the contents of external test programs invoked by
argv. Keep those test sources and their dependencies under independent review
and version control.

The eight assertion kinds cover exit code, stdout/stderr equality or containment,
and file existence/equality/containment. A suite has 1–64 unique cases, with 1–16
explicit assertions per case. JSON rejects unknown fields and duplicate keys.
Expected text is bounded at 64 KiB, normalized suite input at 512 KiB, and host
evidence reads at 1 MiB. File evidence rejects traversal, links and nonregular
files. The per-case 1–600 second budget covers preparation and execution;
fixture preparation before the invocation is separate. Required logs must be
complete UTF-8. Text displayed in the report is limited to 4096 characters per
assertion; complete command logs remain separately linked. Missing declared
output artifacts are known failed assertions.

Haskell entries can call `runTestSuite (suite ... [testCase ...])` with the same
phase/baseline flags. Existing Clef workflows can compose
`runSuite :: FilePath -> Text -> Suite -> Workflow SuiteResult` and
`requireBecause "acceptance failed" suitePassed` to reject an implementation
independently of a provider reply. The core returns structured evidence; the
CLI wrapper adds red/green validation and reports. Place these numbered entries
in `.tactus/motivoscript` so installed helper modules are in the include path.
See the installed `references/tdd.md` and the runnable
[`motivo/examples/tdd`](../motivo/examples/tdd/README.md) example.

Test reports retain `motivo.run/v1`, with `method = "test"` and `test_suite`
metadata, and join the same offline index. `artifacts/suite.json` stores the
oracle; `artifacts/test-suite.json` contains outcomes and independently checked
assertions; `samples.jsonl` retains ordered suite/case/assertion observations.
Existing method and probe records keep their format. Linux acceptance tests
exercise both protocol fixtures and real Bubblewrap red/green; other platforms
compile installed entries and explicitly skip unsupported execution. Passing
assertions establish their declared behavior, not complete correctness of the
user's entire task.

## Records and model identity

Each method writes `.tactus/motivo/runs/<run-id>`. Its request, report, metadata,
samples and artifacts are local business content. Runtime-generated times and
execution results are distinct from agent-authored judgments. Parent references
express known dependencies; timestamps alone do not establish causality.

Provider failures are recorded under `provider_error` in `run.json` and shown
as “Provider failure” in both report formats. The `experiment` field and
“Experiment observation” section are reserved for probe results and failures;
the unused observation field is `null`.

The report records the caller's agent/model labels and any explicit provider
selection. A caller-supplied or requested model is not proof of the model that
actually ran. Missing identity remains “not reported”; Motivo does not guess it
from a CLI name or read credentials to infer an account.

Retrospectives organize expected and observed behavior, supporting evidence,
changes in judgment and the next useful action. Recorded/completed method state
does not mean the user's engineering goal has been independently verified.

## Reading a report

Reports embed their core content and styles. They need no port, server, CDN,
login, browser directory permission, or adjacent JSON fetch. Evidence can be
expanded, and the browser can reload the current file after a new snapshot is
published. Core content is readable with JavaScript disabled.

Section extraction and HTML rendering share code-fence boundaries. Backtick
and tilde fences may have up to three leading spaces; a closing fence must use
the same character at least as many times and contain no trailing text. Headings
inside these blocks remain quoted material rather than report sections.

The page is a snapshot. Its generation time and last observation time describe
what has been recorded; it is not a live connection or proof that an old
in-progress call is still running. The main agent remains the place to request
status and provide direction. Large raw attachments may remain separate files;
the page identifies those rather than claiming they are embedded.

## Migration from the desktop client

The Electron task service, provider UI, execution controls and desktop installers
have been removed. Existing `.motivo/tasks` records are left untouched as
historical material. They are not converted into new samples or automatically
resumed. Tactus's existing control/session APIs and Segno are unchanged.

See [ADR-0008](adr/0008-agent-led-motivo.md) for the ownership decision and
[Observability](observability.md) for the distinction between method reports
and runtime evidence.
