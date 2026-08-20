# LaTeX norm checker

This dependency-free Python program is a reference `agenstro.plugin/v1`
implementation for `agenstro.norm/v1`. It interprets norm data supplied in a
`check` request; no house-style rule is compiled into the checker.

Run its model-free conformance fixtures with:

```powershell
python plugins/latex-norm-check/run_fixtures.py
```

It implements `Existence`, `Absence`, `Occurrence`, `Consistency`, and the
documented `Metric` names. Guidance-only norms, unsupported `kind` values,
unknown metrics, malformed spec shapes, and malformed regular expressions are
returned in `unchecked`; they are never reported as passing. Patterns use
Python `re` with `MULTILINE`, so catalogues must not assume a different regex
dialect.

The entrypoint enforces the repository's strict JSON domain, including
duplicate-key and non-finite/overflow/underflow rejection, and preserves both
string and signed-integer correlation ids.

Clef's default `judge` expects a general plugin registry entry named
`norm-check`. To use the more descriptive name from the Clef guide, register
the script with an absolute path and select it through `judgeWith`:

```toml
[plugins.latex-norm-check]
command = ["python", "D:/src/Agenstro/plugins/latex-norm-check/latex_norm_check.py"]
```

`Sequence`, `ExternalCheck`, and unknown future spec kinds are not implemented
by this checker. Supported metrics are `characters`, `lines`, and
`display-equations`; any other metric is explicitly unchecked.
