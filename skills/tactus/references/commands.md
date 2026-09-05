# Command reference

Pass paths and goals as separate quoted arguments. Tactus searches `--root`
and its parents for `.tactus/tactus.toml`; script paths are relative to the
resolved workspace root. Initialize a new workspace with `tactus init <path>`.

```sh
tactus list --root /path/to/project --json
tactus doctor --root /path/to/project --json
```

`check` takes positional sources, including helper modules. `run` takes
repeatable `--script` arguments and executes numbered entries in discovery
order. Both require an explicit selection: paths, `--all`, or an inclusive
`--from` / `--through` range.

```sh
tactus check --root /path/to/project .tactus/scripts/010_main.hs .tactus/scripts/Support.hs
tactus run --root /path/to/project --script .tactus/scripts/010_main.hs
tactus run --root /path/to/project --script .tactus/scripts/010_main.hs -- 'workflow argument'
```

Both accept `--timeout-seconds N` and repeated `--package NAME`. A timeout of
`0` disables the outer deadline. `--keep-going` continues after a failed entry;
it does not retry it or undo previous effects.

`tactus generate --root /path/to/project 'authoring goal'` invokes the configured
generation provider. It can type-check while authoring; generation does not
execute the resulting business workflow. Edits to helper modules count as
source changes. Select and run entries separately when the task needs execution.

For plugin authoring, read the workspace registry and the plugin's `describe`
response before constructing an invocation. `describe` reports methods and
capabilities; it does not validate task correctness.

```sh
tactus plugin-call --root /path/to/project --namespace effect project.tests describe
tactus plugin-call --root /path/to/project --namespace effect project.tests check --params '{"target":"parser"}'
```

For a custom executable plugin, Tactus sends one `agenstro.plugin/v1` JSONL
request with `id`, `method`, and `params`. Reply with a correlated terminal
`{"type":"result","id":"<request id>","ok":true,"value":...}` or
`{"type":"result","id":"<request id>","ok":false,"error":{"code":"...","message":"..."}}`.
Reserve stdout for protocol frames; use stderr for diagnostics. Check the
repository's plugin protocol reference for optional events and transport limits.
