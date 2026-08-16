# Frozen 0.2 maintenance scripts

The scripts and tests in this directory describe the former `0.2` Python
distribution identities and generated cross-language fixture. They depend on
the removed Clef Python package and are retained only as migration evidence.

They are not part of the Haskell/Tactus `0.3` release gate and are not expected
to run from the current repository root. Current validation lives in:

- `clef-sdk/haskell/test/`;
- `tactus-runtime/tests/`;
- `.github/workflows/haskell-tactus.yml`; and
- the native Rust tests in the legacy workspace.

Do not update an old fixture or identity assertion to define new `0.3`
behavior. If a historical reproduction is needed, run it against a matching
`0.2` checkout or explicitly adapt it under `archive/`.
