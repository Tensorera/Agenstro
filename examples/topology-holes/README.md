# Topology holes: a multi-step Tactus workflow

This example exercises the intended Agenstro workflow from small, independently
checkable steps to a complete command-line program. The target program reads a
rectangular ASCII grid (`#` for foreground and `.` for background) and reports
connected components, holes, and the Euler characteristic.

The digital-topology convention is deliberate:

- foreground components use 4-neighbour connectivity;
- background components use the dual 8-neighbour connectivity;
- a background component is a hole only when it does not touch the grid border;
- `Euler characteristic = foreground components - holes`.

## Workflow stages

Copy the four files in `workflow/` into an initialized project's
`.tactus/scripts/` directory, or pass them explicitly to `tactus check` and
`tactus run`.

1. `010_contract_and_parser.hs` creates only the input contract, parser, and
   parser tests.
2. `020_foreground_components.hs` adds the atomic 4-connected component
   analysis.
3. `030_holes_and_euler.hs` adds dual-connectivity hole counting and Euler
   characteristic tests.
4. `040_integrate_cli.hs` asks two read-only reviewers to inspect the algorithm
   and CLI contract in parallel, then gives their typed findings to an
   integration task that builds and verifies the complete program.

Each entry is an ordinary Haskell program. GHC checks the result wiring between
tasks before any provider is started. Provider progress is reported through the
Clef runtime event sink; only each task's typed terminal JSON enters the workflow
value graph.

The example intentionally does not pin the generated implementation language.
The prompts recommend a strong static type system and a dependency-light CLI.
The checked-in `reference/` happens to use Rust and is a deterministic oracle,
not code that the workflow is expected to copy.

## Offline end-to-end acceptance

The ignored Rust integration test
`haskell_topology_workflow_runs_all_stages_with_parallel_reviews` runs this
example through the real `tactus run -> runghc -> Clef -> tactus dispatch`
path, but substitutes a local protocol fixture for a billed model. The fixture
requires each atomic marker before accepting the next stage, verifies that the
two typed reviews overlap, checks that their findings reach the integration
prompt, writes the final expected summary, and asserts provider/effect journals.
CI invokes this test explicitly on the GHC-enabled acceptance job.

## Deterministic acceptance fixture

`fixtures/two-holes.grid` must produce `fixtures/two-holes.expected.json`. The
reference implementation can be checked without placing Rust artifacts in this
repository:

```powershell
$target = Join-Path $env:TEMP ("agenstro-topology-" + [guid]::NewGuid().ToString("N"))
$env:CARGO_TARGET_DIR = $target
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
try {
  cargo test --manifest-path examples/topology-holes/reference/Cargo.toml
  cargo run --quiet --manifest-path examples/topology-holes/reference/Cargo.toml -- `
    examples/topology-holes/fixtures/two-holes.grid
} finally {
  cargo clean --manifest-path examples/topology-holes/reference/Cargo.toml --target-dir $target
  Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_INCREMENTAL -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_DEV_DEBUG -ErrorAction SilentlyContinue
  Remove-Item Env:CARGO_PROFILE_TEST_DEBUG -ErrorAction SilentlyContinue
}
```

The expected summary is one foreground component, two holes with areas 9 and
9, and Euler characteristic -1.
