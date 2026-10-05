# Command reference

## Script and plugin commands

Pass paths and goals as separate quoted arguments. Tactus searches `--root`
and its parents for `.tactus/tactus.toml`; script paths are relative to the
resolved workspace root. Initialize a new workspace with `tactus init <path>`.

```sh
tactus list --root /path/to/project --json
tactus doctor --root /path/to/project --json
```

For skill upgrades, `tactus skills update --root /path/to/project --dry-run`
previews the installed Tactus, Motivo and Segno files below `.tactus/skills`.
Omit `--dry-run` to apply updates: known unchanged copies are refreshed, while
customized or unknown files remain in place with incoming versions under
`.tactus/skill-updates/` for review. This command does not update business
scripts, method templates or provider configuration; repeated `init` also
preserves existing files.
Neither command installs host discovery mirrors. Read the selected
`.tactus/skills/<name>/SKILL.md` explicitly; existing host directories and
previously installed mirrors remain unchanged.

`check` takes positional sources, including helper modules. `run` takes
repeatable `--script` arguments and preserves their order; `--all` and ranges
use discovery order. Both require an explicit selection: paths, `--all`, or an inclusive
`--from` / `--through` range.

```sh
tactus check --root /path/to/project .tactus/scripts/010_main.hs .tactus/scripts/Support.hs
tactus run --root /path/to/project --script .tactus/scripts/010_main.hs
tactus run --root /path/to/project --script .tactus/scripts/010_main.hs -- 'workflow argument'
```

`list`, `check`, and `run` accept `--scripts-dir .tactus/<directory>` to select
one alternative source tree. Source arguments remain relative to the workspace
root, and Haskell module lookup follows the selected tree. Directories must
exist below `.tactus`; parent traversal and symlink paths are rejected. Without
this option, `--all` only selects `.tactus/scripts`, never Motivo methods.

```sh
tactus list --root /path/to/project --scripts-dir .tactus/motivoscript --json
tactus check --root /path/to/project --scripts-dir .tactus/motivoscript .tactus/motivoscript/020_investigate.hs
tactus run --root /path/to/project --scripts-dir .tactus/motivoscript --script .tactus/motivoscript/020_investigate.hs -- --input findings.md
```

Both accept `--timeout-seconds N` and repeated `--package NAME`. A timeout of
`0` disables the outer deadline. `--keep-going` continues after a failed entry;
it does not retry it or undo previous effects.

The standard `.tactus/segno.hs` organizer is an explicit source exception under
the default source tree; it is not discovered by `--all`. Initialize the Segno
SDK linkage, then expose its package when checking the organizer:

```sh
tactus check --root /path/to/project --package segno-flow .tactus/segno.hs
```

Use the sibling [Segno skill](../../segno/SKILL.md) for its timestamp/result
protocol, execution environment, registration, and recovery. For per-run daemon
supervision and live output, read [supervision.md](supervision.md).

Author the Haskell source in the current coding-agent conversation. For a
provider task, use Clef's `invoke` or `invokeWith` in that source, then check
and run the selected entry. This skill does not permit `tactus generate`,
provider `plugin-call`, `provider-host`, native provider CLIs, `smoke --live`,
or shell/Python/JavaScript wrappers to invoke a provider. See the
[provider calling rules](../SKILL.md#provider-calling-rules).

For plugin authoring, read the workspace registry and the plugin's `describe`
response before constructing an invocation. `describe` reports methods and
capabilities; it does not validate task correctness.
The following commands address an effect, not a model/provider invocation:

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
