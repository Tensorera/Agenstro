"""Project-local Tactus workspace, configuration, and script discovery."""

from __future__ import annotations

import json
import os
import re
import tempfile
import tomllib
from collections.abc import Mapping
from contextlib import suppress
from dataclasses import dataclass
from datetime import date, datetime, time
from pathlib import Path
from typing import cast

from .errors import ConfigurationError, WorkspaceError

CONTROL_DIRECTORY = ".tactus"
CONFIG_NAME = "tactus.toml"
CABAL_PROJECT_NAME = "cabal.project"
PROMPT_NAME = "PROMPT.md"
RUNTIME_CONFIG_NAME = "runtime.json"
SCRIPTS_DIRECTORY = "scripts"
RUNTIME_API = "clef.runtime/v1"
PLUGIN_API = "agenstro.plugin/v1"

_ENTRY_PATTERN = re.compile(
    r"^(?P<order>[0-9]{3})_(?P<slug>[a-z0-9]+(?:_[a-z0-9]+)*)\.(?:hs|lhs)$"
)
_HASKELL_SUFFIXES = {".hs", ".lhs"}

type JsonValue = (
    bool | int | float | str | list[JsonValue] | dict[str, JsonValue] | None
)


@dataclass(frozen=True, slots=True)
class TactusWorkspace:
    """Resolved paths for one initialized project workspace."""

    root: Path
    control: Path
    config_path: Path
    cabal_project_path: Path
    scripts_path: Path
    prompt_path: Path
    runtime_config_path: Path


@dataclass(frozen=True, slots=True)
class InitReport:
    """Files created or deliberately preserved by an idempotent init."""

    workspace: TactusWorkspace
    sdk_path: Path
    created: tuple[str, ...]
    preserved: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class ScriptInfo:
    """One Haskell source discovered under ``.tactus/scripts``."""

    path: Path
    relative_path: str
    kind: str
    order: int | None
    warning: str | None

    @property
    def runnable(self) -> bool:
        """Return whether the filename declares an ordered entry point."""
        return self.kind == "entry"


@dataclass(frozen=True, slots=True)
class LoadedConfig:
    """Validated TOML values needed by the runner and plugin client."""

    api: str
    default_provider: str
    providers: dict[str, dict[str, JsonValue]]
    effects: dict[str, dict[str, JsonValue]]
    instructions_path: Path
    instructions_content: str


DEFAULT_CONFIG = """\
api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus-provider-host", "codex"]

[providers."claude-code"]
command = ["tactus-provider-host", "claude-code"]

[providers.opencode]
command = ["tactus-provider-host", "opencode"]

[effects."workspace.paths"]
command = ["tactus-effect-host", "workspace-paths"]
observe_invocations = true
"""

DEFAULT_PROMPT = """\
# Tactus workflow scripts

- Put generated Haskell workflow entry points in `.tactus/scripts/`.
- Name runnable entry points `NNN_slug.hs` or `NNN_slug.lhs`, using a
  three-digit increasing prefix such as `010_plan.hs`, `020_execute.hs`.
- Tactus runs numbered entry points in numeric order and then by relative path.
- Helper modules may use any Haskell filename and may be nested below the scripts
  directory. Files without the numbered entry convention are never run by
  default, but can be checked or run by explicit path.
- Every runnable entry point is an ordinary command-line Haskell program. Do not
  use Python cell markers or rely on a persistent interpreter session.
- Use the `clef-sdk` package for workflow definitions and route external work
  through the configured provider and effect plugins.
"""


def workspace_paths(root: Path) -> TactusWorkspace:
    """Resolve project-local Tactus paths without requiring initialization."""
    resolved = root.expanduser().resolve()
    control = resolved / CONTROL_DIRECTORY
    return TactusWorkspace(
        root=resolved,
        control=control,
        config_path=control / CONFIG_NAME,
        cabal_project_path=control / CABAL_PROJECT_NAME,
        scripts_path=control / SCRIPTS_DIRECTORY,
        prompt_path=control / PROMPT_NAME,
        runtime_config_path=control / RUNTIME_CONFIG_NAME,
    )


def initialize_workspace(root: Path, *, sdk: Path | None = None) -> InitReport:
    """Create the minimal Tactus layout without overwriting any existing file."""
    workspace = workspace_paths(root)
    sdk_path = resolve_sdk_path(workspace.root, sdk)
    try:
        workspace.root.mkdir(parents=True, exist_ok=True)
        workspace.control.mkdir(parents=True, exist_ok=True)
        workspace.scripts_path.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise WorkspaceError(f"cannot create Tactus workspace: {exc}") from exc
    if not workspace.scripts_path.is_dir():
        raise WorkspaceError(
            f"script path is not a directory: {workspace.scripts_path}"
        )

    files = {
        workspace.config_path: DEFAULT_CONFIG,
        workspace.cabal_project_path: _cabal_project(workspace, sdk_path),
        workspace.prompt_path: DEFAULT_PROMPT,
    }
    created: list[str] = []
    preserved: list[str] = []
    for path, content in files.items():
        relative = path.relative_to(workspace.root).as_posix()
        if _write_new_file(path, content):
            created.append(relative)
        else:
            preserved.append(relative)
    return InitReport(
        workspace=workspace,
        sdk_path=sdk_path,
        created=tuple(created),
        preserved=tuple(preserved),
    )


def open_workspace(root: Path) -> TactusWorkspace:
    """Open an initialized workspace and validate its required layout."""
    workspace = workspace_paths(root)
    missing = [
        path
        for path in (
            workspace.config_path,
            workspace.cabal_project_path,
            workspace.prompt_path,
        )
        if not path.is_file()
    ]
    if not workspace.scripts_path.is_dir():
        missing.append(workspace.scripts_path)
    if missing:
        rendered = ", ".join(str(path) for path in missing)
        raise WorkspaceError(
            f"workspace is not initialized; missing {rendered}. Run `tactus init`."
        )
    return workspace


def resolve_sdk_path(root: Path, supplied: Path | None) -> Path:
    """Resolve an explicit SDK or the sibling SDK in a source checkout."""
    if supplied is not None:
        candidate = supplied.expanduser()
        if not candidate.is_absolute():
            candidate = root / candidate
        return _existing_directory(candidate, "Clef SDK")

    candidates: list[Path] = []
    environment = os.environ.get("TACTUS_CLEF_SDK")
    if environment:
        candidates.append(Path(environment).expanduser())
    candidates.extend((root / "clef-sdk", root.parent / "clef-sdk"))
    source = Path(__file__).resolve()
    if len(source.parents) > 3:
        candidates.append(source.parents[3] / "clef-sdk")
    for candidate in candidates:
        try:
            if candidate.is_dir():
                return candidate.resolve()
        except OSError:
            continue
    raise WorkspaceError(
        "cannot locate the sibling clef-sdk; pass `tactus init --sdk PATH`"
    )


def discover_scripts(workspace: TactusWorkspace) -> tuple[ScriptInfo, ...]:
    """Discover numbered entries and arbitrary helper modules deterministically."""
    try:
        paths = [
            path
            for path in workspace.scripts_path.rglob("*")
            if path.is_file() and path.suffix.lower() in _HASKELL_SUFFIXES
        ]
    except OSError as exc:
        raise WorkspaceError(f"cannot enumerate Haskell scripts: {exc}") from exc

    entries: list[ScriptInfo] = []
    helpers: list[ScriptInfo] = []
    for path in paths:
        resolved = path.resolve()
        relative = path.relative_to(workspace.root).as_posix()
        match = _ENTRY_PATTERN.fullmatch(path.name)
        if match is not None:
            entries.append(
                ScriptInfo(
                    path=resolved,
                    relative_path=relative,
                    kind="entry",
                    order=int(match.group("order")),
                    warning=None,
                )
            )
        else:
            helpers.append(
                ScriptInfo(
                    path=resolved,
                    relative_path=relative,
                    kind="helper",
                    order=None,
                    warning=(
                        "filename does not match NNN_slug.hs or NNN_slug.lhs; "
                        "treated as a helper"
                    ),
                )
            )
    entries.sort(
        key=lambda item: (
            item.order or 0,
            item.relative_path.casefold(),
            item.relative_path,
        )
    )
    helpers.sort(key=lambda item: (item.relative_path.casefold(), item.relative_path))
    return (*entries, *helpers)


def explicit_scripts(
    workspace: TactusWorkspace, values: list[Path]
) -> tuple[Path, ...]:
    """Resolve explicitly selected Haskell files without imposing a root boundary."""
    resolved: list[Path] = []
    for value in values:
        candidate = value.expanduser()
        if not candidate.is_absolute():
            candidate = workspace.root / candidate
        try:
            candidate = candidate.resolve(strict=True)
        except OSError as exc:
            raise WorkspaceError(f"cannot resolve script {value}: {exc}") from exc
        if not candidate.is_file() or candidate.suffix.lower() not in _HASKELL_SUFFIXES:
            raise WorkspaceError(f"not a Haskell source file: {candidate}")
        resolved.append(candidate)
    return tuple(resolved)


def load_config(workspace: TactusWorkspace) -> LoadedConfig:
    """Read and validate the language-neutral project TOML."""
    try:
        with workspace.config_path.open("rb") as stream:
            raw = cast(dict[str, object], tomllib.load(stream))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise ConfigurationError(f"cannot read {workspace.config_path}: {exc}") from exc

    api = _text(raw.get("api"), "api")
    default_provider = _text(raw.get("default_provider"), "default_provider")
    if api != RUNTIME_API:
        raise ConfigurationError(f"api must be {RUNTIME_API!r}, received {api!r}")
    providers = _plugins(raw.get("providers"), "providers", required=True)
    effects = _plugins(raw.get("effects", {}), "effects", required=False)
    if default_provider not in providers:
        raise ConfigurationError(
            f"default_provider {default_provider!r} is not defined in providers"
        )
    instructions_value = _text(raw.get("instructions"), "instructions")
    instructions_path = Path(instructions_value).expanduser()
    if not instructions_path.is_absolute():
        instructions_path = workspace.root / instructions_path
    try:
        instructions_path = instructions_path.resolve(strict=True)
        instructions_content = instructions_path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        raise ConfigurationError(f"cannot read instructions: {exc}") from exc
    return LoadedConfig(
        api=api,
        default_provider=default_provider,
        providers=providers,
        effects=effects,
        instructions_path=instructions_path,
        instructions_content=instructions_content,
    )


def write_runtime_config(workspace: TactusWorkspace) -> dict[str, JsonValue]:
    """Normalize TOML and resolved instructions into canonical runtime JSON."""
    config = load_config(workspace)
    providers: dict[str, JsonValue] = {
        name: _normalize_provider(plugin) for name, plugin in config.providers.items()
    }
    effects: dict[str, JsonValue] = {
        name: _normalize_effect(plugin) for name, plugin in config.effects.items()
    }
    value: dict[str, JsonValue] = {
        "api": RUNTIME_API,
        "workspace": str(workspace.root),
        "default_provider": config.default_provider,
        "providers": providers,
        "effects": effects,
        "instructions": config.instructions_content,
    }
    try:
        encoded = (
            json.dumps(
                value,
                allow_nan=False,
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            )
            + "\n"
        )
    except (TypeError, ValueError) as exc:
        raise ConfigurationError(f"runtime config is not valid JSON: {exc}") from exc
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            dir=workspace.runtime_config_path.parent,
            prefix=f".{RUNTIME_CONFIG_NAME}.",
            suffix=".tmp",
            delete=False,
        ) as stream:
            temporary_path = Path(stream.name)
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, workspace.runtime_config_path)
        temporary_path = None
    except OSError as exc:
        raise ConfigurationError(f"cannot write runtime config: {exc}") from exc
    finally:
        if temporary_path is not None:
            with suppress(OSError):
                temporary_path.unlink()
    return value


def plugin_commands(
    config: LoadedConfig,
) -> tuple[tuple[str, str, tuple[str, ...]], ...]:
    """Return all configured plugin argv in stable provider/effect order."""
    values: list[tuple[str, str, tuple[str, ...]]] = []
    for kind, plugins in (("provider", config.providers), ("effect", config.effects)):
        for name in sorted(plugins, key=str.casefold):
            command = _command(
                plugins[name].get("command"),
                f"{kind} {name!r} command",
            )
            values.append((kind, name, command))
    return tuple(values)


def runtime_environment(workspace: TactusWorkspace) -> dict[str, str]:
    """Inherit the caller environment and point children at normalized config."""
    environment = os.environ.copy()
    environment["TACTUS_RUNTIME_CONFIG"] = str(workspace.runtime_config_path.resolve())
    return environment


def _plugins(
    value: object,
    location: str,
    *,
    required: bool,
) -> dict[str, dict[str, JsonValue]]:
    if not isinstance(value, dict):
        raise ConfigurationError(f"{location} must be a TOML table")
    table = cast(Mapping[object, object], value)
    if required and not table:
        raise ConfigurationError(f"{location} must be a non-empty TOML table")
    result: dict[str, dict[str, JsonValue]] = {}
    for name, raw in table.items():
        if not isinstance(name, str) or not name or not isinstance(raw, dict):
            raise ConfigurationError(f"{location} entries must be named TOML tables")
        converted = _json_value(
            cast(dict[object, object], raw),
            f"{location}.{name}",
        )
        if not isinstance(converted, dict):
            raise ConfigurationError(f"{location}.{name} must be a TOML table")
        command = converted.get("command")
        if (
            not isinstance(command, list)
            or not command
            or not all(isinstance(item, str) and item for item in command)
        ):
            raise ConfigurationError(
                f"{location}.{name}.command must be a non-empty argv array"
            )
        result[name] = converted
    return result


def _normalize_provider(plugin: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "command": plugin["command"],
        "model": _optional_text(plugin.get("model"), "provider.model"),
        "effort": _optional_text(plugin.get("effort"), "provider.effort"),
        "options": _object(plugin.get("options", {}), "provider.options"),
    }


def _normalize_effect(plugin: dict[str, JsonValue]) -> dict[str, JsonValue]:
    observed = plugin.get("observe_invocations", False)
    if not isinstance(observed, bool):
        raise ConfigurationError("effect.observe_invocations must be boolean")
    return {
        "command": plugin["command"],
        "options": _object(plugin.get("options", {}), "effect.options"),
        "observe_invocations": observed,
    }


def _optional_text(value: JsonValue, location: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise ConfigurationError(f"{location} must be null or non-empty text")
    return value


def _object(value: JsonValue, location: str) -> dict[str, JsonValue]:
    if not isinstance(value, dict):
        raise ConfigurationError(f"{location} must be a table")
    return value


def _json_value(value: object, location: str) -> JsonValue:
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (datetime, date, time)):
        return value.isoformat()
    if isinstance(value, list):
        items = cast(list[object], value)
        return [_json_value(item, location) for item in items]
    if isinstance(value, dict):
        items = cast(Mapping[object, object], value)
        converted: dict[str, JsonValue] = {}
        for key, item in items.items():
            if not isinstance(key, str):
                raise ConfigurationError(f"{location} contains a non-text key")
            converted[key] = _json_value(item, f"{location}.{key}")
        return converted
    raise ConfigurationError(f"{location} contains an unsupported TOML value")


def _text(value: object, location: str) -> str:
    if not isinstance(value, str) or not value:
        raise ConfigurationError(f"{location} must be non-empty text")
    return value


def _command(value: JsonValue, location: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not value:
        raise ConfigurationError(f"{location} must be a non-empty argv array")
    command: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item:
            raise ConfigurationError(f"{location} must be a non-empty argv array")
        command.append(item)
    return tuple(command)


def _existing_directory(path: Path, label: str) -> Path:
    try:
        resolved = path.resolve(strict=True)
    except OSError as exc:
        raise WorkspaceError(f"{label} is unavailable: {path}") from exc
    if not resolved.is_dir():
        raise WorkspaceError(f"{label} is not a directory: {resolved}")
    return resolved


def _write_new_file(path: Path, content: str) -> bool:
    try:
        with path.open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(content)
    except FileExistsError:
        if not path.is_file():
            raise WorkspaceError(f"cannot preserve non-file path: {path}") from None
        return False
    except OSError as exc:
        raise WorkspaceError(f"cannot create {path}: {exc}") from exc
    return True


def _cabal_project(workspace: TactusWorkspace, sdk: Path) -> str:
    try:
        relative = os.path.relpath(sdk, workspace.control)
    except ValueError:
        relative = str(sdk)
    portable = Path(relative).as_posix()
    quoted = json.dumps(portable, ensure_ascii=False)
    return f"packages:\n  {quoted}\n"


__all__ = [
    "CABAL_PROJECT_NAME",
    "CONFIG_NAME",
    "CONTROL_DIRECTORY",
    "PLUGIN_API",
    "PROMPT_NAME",
    "RUNTIME_API",
    "RUNTIME_CONFIG_NAME",
    "SCRIPTS_DIRECTORY",
    "InitReport",
    "LoadedConfig",
    "ScriptInfo",
    "TactusWorkspace",
    "discover_scripts",
    "explicit_scripts",
    "initialize_workspace",
    "load_config",
    "open_workspace",
    "plugin_commands",
    "resolve_sdk_path",
    "runtime_environment",
    "workspace_paths",
    "write_runtime_config",
]
