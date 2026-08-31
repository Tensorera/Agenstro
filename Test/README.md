# Repository tests and cases

`Test/` holds repository-level material that does not belong to one product
package:

- `fixtures/` contains cross-language contract inputs; and
- `repository/` contains PowerShell checks for repository-wide publication
  contracts.

Tests that exercise one implementation stay beside that implementation:
`clef-sdk/haskell/test/`, `tactus-runtime/tests-rust/`,
`segno-flow/haskell/test/`, and `motivo-studio/tests/`. This keeps normal package
tools and IDE discovery working without custom test-path rewrites.

The language-neutral norm checker keeps its request/expectation fixtures beside
the implementation under `plugins/latex-norm-check/fixtures/`; run them with
`python plugins/latex-norm-check/run_fixtures.py`.

The canonical repository entrypoint composes these package and publication
checks without contacting a live provider:

```powershell
./scripts/quality.ps1 -Profile Fast
./scripts/quality.ps1 -Profile Full
```

Generated files never belong under `Test/`. Cargo, Cabal, MkDocs, and Electron
Forge recreate their repository-level output under the ignored `Build/`
directory.
