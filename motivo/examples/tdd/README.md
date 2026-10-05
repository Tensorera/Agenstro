# A real local red/green example

Use an installed Tactus/Clef/Motivo workspace and the Linux `motivo-test`
backend. This example calls no model. The sandbox reads `/project/add.py` and
cannot modify it. The coding agent or user makes the implementation change
between the two explicit invocations.

Run in a temporary workspace so the example does not modify your project:

```sh
example_root=/absolute/path/to/Agenstro/motivo/examples/tdd
example_project=$(mktemp -d)
tactus init "$example_project" --sdk /absolute/path/to/Agenstro/clef-sdk
cp "$example_root/add.py" "$example_project/add.py"
cp "$example_root/suite.json" "$example_project/acceptance.json"
cd "$example_project"

tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/090_test.hs \
  -- --suite acceptance.json --run-id addition-red --phase red
# Exit 1 is expected: subtraction fails both addition assertions.

cp "$example_root/fixed-add.py" add.py

tactus run --scripts-dir .tactus/motivoscript \
  --script .tactus/motivoscript/090_test.hs \
  -- --suite acceptance.json --run-id addition-green \
  --phase green --baseline addition-red
# Exit 0 is expected. The same frozen acceptance now passes.
```

Open the printed reports or `.tactus/motivo/index.html`. Their JSON artifacts
preserve the command facts, assertions and red baseline link. A lowered expected
value or changed command is rejected before a green execution. Unavailable
isolation, timeout, incomplete logs or protocol failures return 2; they do not
count as the required red observation.

The equivalent Haskell entry is `100_acceptance.hs`. Copy it into
`.tactus/motivoscript` and replace the JSON entry/`--suite` argument with that
script to use the same CLI cycle. Its `verifyImplementation` function shows the
typed `runSuite`/`requireBecause` composition available to an existing Clef
workflow. The core returns structured evidence; the entrypoint adds Motivo
reports and phase/baseline validation.

The repository acceptance test runs this closed loop in its own temporary folder:

```sh
python3 motivo/tests/tdd.py --tactus Build/cargo/debug/tactus \
  --motivo-test Build/cargo/debug/motivo-test --installed-only
```

The suite is an acceptance oracle under the caller's control. Baseline matching
helps preserve that oracle across implementation changes; it is not a signature
or protection from a caller editing its evidence files.
