# Command acceptance and a recorded red/green cycle

Use this when the user wants executable acceptance before implementation. The
existing coding agent writes the suite, edits the implementation, and decides
what to do next. Motivo executes one bounded command per case through
`motivo.test` and checks its facts independently. No provider is called by the
test entrypoint. A provider's `testsRun` field is not an assertion.

The backend is currently Linux with usable Bubblewrap. If isolation is
unavailable, the suite records an error; do not substitute an ordinary shell or
`liftIO` payload.

## Write the acceptance oracle first

Create `acceptance.json` in the initialized workspace:

```json
{
  "api": "motivo.test-suite/v1",
  "id": "addition",
  "cases": [
    {
      "id": "two-plus-two",
      "command": {
        "argv": ["/usr/bin/python3", "/project/add.py", "2", "2"],
        "timeout_seconds": 15
      },
      "assertions": [
        {"kind": "exit_code", "expected": 0},
        {"kind": "stdout_equals", "expected": "4\n"}
      ]
    }
  ]
}
```

The JSON boundary rejects duplicate keys and unknown fields. A suite has 1–64
cases, unique case ids, and 1–16 explicit assertions per case. Suite/case/run ids
are 1–80 ASCII letters, digits, underscores or hyphens. Each command is a literal
argv array, not implicit shell input. Its timeout is 1–600 seconds, default 600,
covering preparation and execution of that case. Cases execute once,
sequentially; there is no automatic retry or implementation loop.

Assertion kinds are `exit_code`, `stdout_equals`, `stdout_contains`,
`stderr_equals`, `stderr_contains`, `file_exists`, `file_equals`, and
`file_contains`. Text assertions use `expected`; file assertions use a `path`
relative to `/work`. Equality preserves newlines. Contains needs nonempty text.
File assertions reject traversal, symbolic/hard links and nonregular evidence.
Expected text is at most 64 KiB; the normalized suite is at most 512 KiB.

Project inputs are read-only at `/project`; persistent writes stay in
`.tactus/motivotest/<run-id>/<case-id>` at `/work`. Prepare fixtures there if
needed. Host-side evidence text reads are bounded at 1 MiB. Required logs must
be present, complete UTF-8 and untruncated. Timeout, cancellation, signal
termination, malformed facts, missing logs and unsafe paths are errors, not
ordinary red tests. Missing declared output files are actual failed assertions.

## Observe red, change the implementation, observe green

```sh
tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/090_test.hs \
  -- --suite acceptance.json --run-id addition-red --phase red

# Make the authorized implementation change outside the sandbox.

tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/090_test.hs \
  -- --suite acceptance.json --run-id addition-green \
  --phase green --baseline addition-red
```

An observed red phase returns **1** because at least one assertion actually
failed, with no execution/evidence errors. Red where every assertion passes
returns **2**: it did not establish a red observation. Green requires a valid
red baseline with the exact normalized suite, including argv, timeouts, case
identity/order and every assertion. A changed oracle is rejected before
execution. Implementation contents may change between the two runs.

Normal and green return **0** when every case passes, **1** on actual assertion
failure, and **2** for invalid input, phase/baseline, or unusable execution
evidence. Each run id is new. Existing reports and sandbox samples are preserved.
Inspect an interrupted run before choosing a new identity; do not treat it as
red or retry it automatically.

The baseline is caller-controlled local evidence, not a signature or protection
against a user rewriting reports. Keep the oracle under the project's review
and version-control policy.

The manifest does not freeze the contents of external test programs invoked by
argv. Keep those test sources and their dependencies under independent review
and version control.

## Haskell and workflow composition

Place an authored entry in `.tactus/motivoscript/100_acceptance.hs`:

```haskell
{-# LANGUAGE OverloadedStrings #-}
import Motivo.Test

main :: IO ()
main = runTestSuite (suite "addition"
  [testCase "two-plus-two"
    (withTimeout 15 (command ["/usr/bin/python3", "/project/add.py", "2", "2"]))
    [assertExit 0, assertStdout "4\n"]])
```

Run it with the same phase/run/baseline options, without `--suite`. Numbered
entry names follow Tactus's `NNN_slug.hs` contract. Installed helper modules
are found via `--scripts-dir .tactus/motivoscript`.

Existing Clef workflows can use
`runSuite :: FilePath -> Text -> Suite -> Workflow SuiteResult`, inspect
`suitePassed`, `suiteStatus`, `resultJSON`, and use `requireBecause` to stop
after failed independent acceptance. `runSuite` returns typed evidence through
the workflow; `runTestSuite` adds CLI red/green checks and Motivo reports. Use
the workspace root and a fresh run id. Keep provider selection explicit with
`invokeWith`; testing does not select one.

## Read the evidence

The CLI prints `.tactus/motivo/runs/<run-id>/report.html` and the shared index.
`run.json` keeps `motivo.run/v1`, with `method = "test"` and a `test_suite`
field. `artifacts/suite.json` preserves the normalized oracle.
`artifacts/test-suite.json` contains case outcomes and host-evaluated
assertions; `samples.jsonl` records ordered observations. Full logs remain in
the protected sample log directory. HTML embeds results and escapes text,
with no execution controls.

Acceptance proves the declared assertions for the executed commands. Preserve
uncovered behavior and unknown outcomes in the calling agent's report.
