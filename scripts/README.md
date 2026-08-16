# Maintenance scripts

`check_workspace_boundaries.py` validates the retained shared Rust crates and
Segno dependency direction. Its regression tests remain current.

The other scripts and tests in this directory describe the former `0.2` Python
distribution identities and generated cross-language fixture. They depend on
the removed Clef Python package and are retained only as migration evidence.

They are not part of the Haskell/Tactus `0.3` release gate and are not expected
to run from the current repository root. Current validation lives in:

- `clef-sdk/haskell/test/`;
- `tactus-runtime/tests/`;
- `.github/workflows/haskell-tactus.yml`; and
- the native tests for the retained Rust foundation and Segno.

Do not update an old fixture or identity assertion to define new `0.3`
behavior. If a historical reproduction is needed, run it against a matching
`0.2` checkout or explicitly adapt it under `archive/`.
