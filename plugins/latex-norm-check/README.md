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
dialect. Regex compilation and matching run in a short-lived worker with a
one-second deadline. This portable process boundary can actually terminate
catastrophic backtracking on Windows and Unix without another regex package;
a valid but unusually expensive pattern is conservatively reported as
`unchecked`.

The checker accepts at most 512 KiB of UTF-8 source and 4 KiB per UTF-8
pattern. A bound needs at least one non-null endpoint. Every `Consistency`
group needs at least two distinct, non-empty patterns. The complete plugin
request is capped at 1 MiB. These limits match Clef's norm-v1 validation and
leave room for catalogue metadata inside the generic transport envelope.

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
