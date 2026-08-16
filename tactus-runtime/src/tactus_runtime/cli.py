"""Command-line workspace and Haskell script runner for Tactus."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from uuid import uuid4

from . import __version__
from .errors import ConfigurationError, PluginProtocolError, TactusRuntimeError
from .plugin_client import PluginExecutor, PluginResponse, invoke_plugin
from .runner import (
    CommandExecutor,
    ToolLocator,
    check_scripts,
    effective_path,
    run_scripts,
    tool_diagnostics,
)
from .workspace import (
    JsonValue,
    ScriptInfo,
    TactusWorkspace,
    discover_scripts,
    explicit_scripts,
    initialize_workspace,
    load_config,
    open_workspace,
    plugin_commands,
    runtime_environment,
    workspace_paths,
    write_runtime_config,
)


@dataclass(frozen=True, slots=True)
class _ObservedInvocation:
    response: PluginResponse | None
    evidence: tuple[dict[str, JsonValue], ...]
    observer_errors: tuple[dict[str, JsonValue], ...]
    error: str | None
    outcome: str


def build_parser() -> argparse.ArgumentParser:
    """Build the small project-local Tactus command surface."""
    parser = argparse.ArgumentParser(
        prog="tactus",
        description="Initialize, check, and run Haskell workflow scripts.",
    )
    parser.add_argument(
        "--version", action="version", version=f"%(prog)s {__version__}"
    )
    commands = parser.add_subparsers(dest="command", required=True)

    init = commands.add_parser("init", help="initialize a .tactus workspace")
    init.add_argument("root", nargs="?", type=Path, default=Path.cwd())
    init.add_argument("--root", dest="root_option", type=Path)
    init.add_argument("--sdk", type=Path, help="path to the clef-sdk Cabal package")
    init.add_argument("--json", action="store_true", help="emit machine-readable JSON")

    listing = commands.add_parser("list", help="list Haskell entries and helpers")
    _root_argument(listing)
    listing.add_argument(
        "--json", action="store_true", help="emit machine-readable JSON"
    )

    prompt = commands.add_parser("prompt", help="print the workspace generation prompt")
    _root_argument(prompt)

    generate = commands.add_parser(
        "generate",
        help="ask a configured provider to create Haskell workflow scripts",
    )
    _root_argument(generate)
    generate.add_argument("goal", nargs="+", help="workflow generation goal")
    generate.add_argument("--provider", help="provider name (defaults to config)")
    generate.add_argument(
        "--json", action="store_true", help="emit machine-readable JSON"
    )

    check = commands.add_parser("check", help="type-check Haskell scripts with GHC")
    _root_argument(check)
    check.add_argument("scripts", nargs="*", type=Path)
    check.add_argument("--keep-going", action="store_true")

    run = commands.add_parser("run", help="run ordered Haskell entry scripts")
    _root_argument(run)
    run.add_argument("scripts", nargs="*", type=Path)
    run.add_argument("--keep-going", action="store_true")

    doctor = commands.add_parser("doctor", help="diagnose workspace and Haskell tools")
    _root_argument(doctor)
    doctor.add_argument(
        "--json", action="store_true", help="emit machine-readable JSON"
    )

    smoke = commands.add_parser("smoke", help="call configured plugin smoke methods")
    _root_argument(smoke)
    smoke.add_argument("plugins", nargs="*")
    smoke.add_argument(
        "--live", action="store_true", help="allow a live provider probe"
    )
    smoke.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    return parser


def main(
    argv: Sequence[str] | None = None,
    *,
    locator: ToolLocator | None = None,
    command_executor: CommandExecutor | None = None,
    plugin_executor: PluginExecutor | None = None,
) -> int:
    """Run one CLI command and return its process exit code."""
    values = list(sys.argv[1:] if argv is None else argv)
    parse_values, script_arguments = _split_script_arguments(values)
    arguments = build_parser().parse_args(parse_values)
    if script_arguments and arguments.command != "run":
        build_parser().error("arguments after `--` are supported only by `tactus run`")

    try:
        if arguments.command == "init":
            return _init(arguments)
        if arguments.command == "doctor":
            return _doctor(arguments, locator=locator)

        workspace = open_workspace(arguments.root)
        if arguments.command == "list":
            return _list(workspace, as_json=arguments.json)
        if arguments.command == "prompt":
            return _prompt(workspace)
        if arguments.command == "generate":
            return _generate(
                workspace,
                " ".join(arguments.goal),
                provider=arguments.provider,
                as_json=arguments.json,
                plugin_executor=plugin_executor,
            )
        if arguments.command == "check":
            scripts = _selected_scripts(
                workspace, arguments.scripts, entries_only=False
            )
            if command_executor is None:
                return check_scripts(
                    workspace,
                    scripts,
                    keep_going=arguments.keep_going,
                    locator=locator,
                )
            return check_scripts(
                workspace,
                scripts,
                keep_going=arguments.keep_going,
                locator=locator,
                executor=command_executor,
            )
        if arguments.command == "run":
            scripts = _selected_scripts(workspace, arguments.scripts, entries_only=True)
            if command_executor is None:
                return run_scripts(
                    workspace,
                    scripts,
                    script_arguments,
                    keep_going=arguments.keep_going,
                    locator=locator,
                )
            return run_scripts(
                workspace,
                scripts,
                script_arguments,
                keep_going=arguments.keep_going,
                locator=locator,
                executor=command_executor,
            )
        return _smoke(
            workspace,
            arguments.plugins,
            live=arguments.live,
            as_json=arguments.json,
            plugin_executor=plugin_executor,
        )
    except TactusRuntimeError as exc:
        print(f"tactus: {exc}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("tactus: interrupted", file=sys.stderr)
        return 130


def _init(arguments: argparse.Namespace) -> int:
    root = arguments.root if arguments.root_option is None else arguments.root_option
    sdk = arguments.sdk
    if sdk is not None and not sdk.is_absolute():
        sdk = (Path.cwd() / sdk).resolve()
    report = initialize_workspace(root, sdk=sdk)
    value = {
        "workspace": str(report.workspace.root),
        "sdk": str(report.sdk_path),
        "created": list(report.created),
        "preserved": list(report.preserved),
    }
    if arguments.json:
        _print_json(value)
    else:
        print(f"Tactus workspace: {report.workspace.root}")
        for path in report.created:
            print(f"created   {path}")
        for path in report.preserved:
            print(f"preserved {path}")
    return 0


def _list(workspace: TactusWorkspace, *, as_json: bool) -> int:
    scripts = discover_scripts(workspace)
    if as_json:
        _print_json(
            {
                "workspace": str(workspace.root),
                "scripts": [_script_value(script) for script in scripts],
            }
        )
        return 0
    _print_script_rows(scripts)
    return 0


def _prompt(workspace: TactusWorkspace) -> int:
    config = load_config(workspace)
    ending = "" if config.instructions_content.endswith("\n") else "\n"
    print(config.instructions_content, end=ending)
    return 0


def _generate(
    workspace: TactusWorkspace,
    goal: str,
    *,
    provider: str | None,
    as_json: bool,
    plugin_executor: PluginExecutor | None,
) -> int:
    runtime = write_runtime_config(workspace)
    provider_name = provider or _required_runtime_text(
        runtime.get("default_provider"),
        "default_provider",
    )
    providers = runtime.get("providers")
    if not isinstance(providers, dict) or provider_name not in providers:
        raise ConfigurationError(f"unknown provider {provider_name!r}")
    selected = providers[provider_name]
    if not isinstance(selected, dict):
        raise ConfigurationError(f"provider {provider_name!r} is invalid")
    command = _required_runtime_command(selected.get("command"), provider_name)
    model = selected.get("model")
    effort = selected.get("effort")
    options = selected.get("options")
    if model is not None and not isinstance(model, str):
        raise ConfigurationError(f"provider {provider_name!r} model is invalid")
    if effort is not None and not isinstance(effort, str):
        raise ConfigurationError(f"provider {provider_name!r} effort is invalid")
    if not isinstance(options, dict):
        raise ConfigurationError(f"provider {provider_name!r} options are invalid")

    generation_prompt = _generation_prompt(
        _required_runtime_text(runtime.get("instructions"), "instructions"),
        goal,
    )
    environment = runtime_environment(workspace)
    environment["PATH"] = effective_path()
    params: dict[str, JsonValue] = {
        "prompt": generation_prompt,
        "workspace": str(workspace.root),
        "model": model,
        "effort": effort,
        "options": options,
    }
    observed = _invoke_with_observers(
        runtime,
        workspace,
        provider_name=provider_name,
        goal=goal,
        provider_command=command,
        provider_params=params,
        environment=environment,
        plugin_executor=plugin_executor,
    )
    scripts = discover_scripts(workspace)
    if observed.response is None:
        value = {
            "provider": provider_name,
            "ok": False,
            "value": None,
            "error": {
                "code": observed.outcome,
                "message": observed.error,
            },
            "events": [],
            "effects": list(observed.evidence),
            "observer_errors": list(observed.observer_errors),
            "exit_code": None,
            "scripts": [_script_value(script) for script in scripts],
        }
        if as_json:
            _print_json(value)
        else:
            print(f"provider {provider_name}: failed")
            print(f"  {observed.error}", file=sys.stderr)
            for observer_error in observed.observer_errors:
                print(f"  observer failure: {observer_error}", file=sys.stderr)
            _print_script_rows(scripts)
        return 1

    response = observed.response
    effect_evidence = list(observed.evidence)
    observer_errors = list(observed.observer_errors)
    succeeded = response.ok and response.exit_code == 0 and not observer_errors
    value = {
        "provider": provider_name,
        "ok": succeeded,
        "value": response.value,
        "error": response.error,
        "events": list(response.events),
        "effects": effect_evidence,
        "observer_errors": observer_errors,
        "exit_code": response.exit_code,
        "scripts": [_script_value(script) for script in scripts],
    }
    if as_json:
        _print_json(value)
    else:
        state = "ok" if succeeded else "failed"
        print(f"provider {provider_name}: {state}")
        if response.error is not None:
            print(f"  {response.error}", file=sys.stderr)
        for observer_error in observer_errors:
            print(f"  observer failure: {observer_error}", file=sys.stderr)
        _print_script_rows(scripts)
    return 0 if succeeded else 1


def _invoke_with_observers(
    runtime: dict[str, JsonValue],
    workspace: TactusWorkspace,
    *,
    provider_name: str,
    goal: str,
    provider_command: Sequence[str],
    provider_params: dict[str, JsonValue],
    environment: dict[str, str],
    plugin_executor: PluginExecutor | None,
) -> _ObservedInvocation:
    invocation = str(uuid4())
    context: dict[str, JsonValue] = {
        "source": "tactus.generate",
        "provider": provider_name,
        "goal": goal,
    }
    evidence: list[dict[str, JsonValue]] = []
    active: list[
        tuple[str, tuple[str, ...], dict[str, JsonValue], JsonValue | None]
    ] = []

    try:
        for effect_name, effect_command, effect_options in _runtime_observers(runtime):
            begin = _call_plugin(
                effect_command,
                method="observe.begin",
                params={
                    "workspace": str(workspace.root),
                    "options": effect_options,
                    "invocation": invocation,
                    "context": context,
                },
                workspace=workspace,
                environment=environment,
                plugin_executor=plugin_executor,
            )
            if not begin.ok or begin.exit_code != 0:
                raise PluginProtocolError(
                    f"effect {effect_name!r} observe.begin failed: {begin.error!r}"
                )
            evidence.append(_effect_record(effect_name, "observe.begin", begin))
            active.append((effect_name, effect_command, effect_options, begin.value))
    except BaseException:
        if active:
            _end_observers(
                active,
                workspace=workspace,
                invocation=invocation,
                context=context,
                outcome="begin_error",
                environment=environment,
                plugin_executor=plugin_executor,
                evidence=evidence,
            )
        raise

    try:
        provider_response = _call_plugin(
            provider_command,
            method="invoke",
            params=provider_params,
            workspace=workspace,
            environment=environment,
            plugin_executor=plugin_executor,
        )
    except TactusRuntimeError as exc:
        observer_errors = _end_observers(
            active,
            workspace=workspace,
            invocation=invocation,
            context=context,
            outcome="outcome_unknown",
            environment=environment,
            plugin_executor=plugin_executor,
            evidence=evidence,
        )
        return _ObservedInvocation(
            response=None,
            evidence=tuple(evidence),
            observer_errors=tuple(observer_errors),
            error=str(exc),
            outcome="outcome_unknown",
        )
    except BaseException:
        _end_observers(
            active,
            workspace=workspace,
            invocation=invocation,
            context=context,
            outcome="interrupted",
            environment=environment,
            plugin_executor=plugin_executor,
            evidence=evidence,
        )
        raise

    provider_succeeded = provider_response.ok and provider_response.exit_code == 0
    observer_errors = _end_observers(
        active,
        workspace=workspace,
        invocation=invocation,
        context=context,
        outcome="ok" if provider_succeeded else "error",
        environment=environment,
        plugin_executor=plugin_executor,
        evidence=evidence,
    )
    return _ObservedInvocation(
        response=provider_response,
        evidence=tuple(evidence),
        observer_errors=tuple(observer_errors),
        error=None,
        outcome="ok" if provider_succeeded else "error",
    )


def _runtime_observers(
    runtime: dict[str, JsonValue],
) -> tuple[tuple[str, tuple[str, ...], dict[str, JsonValue]], ...]:
    effects = runtime.get("effects")
    if not isinstance(effects, dict):
        raise ConfigurationError("runtime effects registry is invalid")
    observers: list[tuple[str, tuple[str, ...], dict[str, JsonValue]]] = []
    for effect_name in sorted(effects, key=str.casefold):
        configured = effects[effect_name]
        if not isinstance(configured, dict):
            raise ConfigurationError(f"runtime effect {effect_name!r} is invalid")
        observed = configured.get("observe_invocations")
        if not isinstance(observed, bool):
            raise ConfigurationError(
                f"runtime effect {effect_name!r} observe_invocations is invalid"
            )
        if not observed:
            continue
        options = configured.get("options")
        if not isinstance(options, dict):
            raise ConfigurationError(
                f"runtime effect {effect_name!r} options are invalid"
            )
        observers.append(
            (
                effect_name,
                _required_runtime_command(configured.get("command"), effect_name),
                options,
            )
        )
    return tuple(observers)


def _end_observers(
    active: Sequence[
        tuple[str, tuple[str, ...], dict[str, JsonValue], JsonValue | None]
    ],
    *,
    workspace: TactusWorkspace,
    invocation: str,
    context: dict[str, JsonValue],
    outcome: str,
    environment: dict[str, str],
    plugin_executor: PluginExecutor | None,
    evidence: list[dict[str, JsonValue]],
) -> list[dict[str, JsonValue]]:
    errors: list[dict[str, JsonValue]] = []
    for effect_name, command, options, begin_value in reversed(active):
        try:
            response = _call_plugin(
                command,
                method="observe.end",
                params={
                    "workspace": str(workspace.root),
                    "options": options,
                    "invocation": invocation,
                    "context": context,
                    "outcome": outcome,
                    "begin": begin_value,
                },
                workspace=workspace,
                environment=environment,
                plugin_executor=plugin_executor,
            )
        except TactusRuntimeError as exc:
            errors.append(
                {
                    "effect": effect_name,
                    "method": "observe.end",
                    "error": str(exc),
                }
            )
            continue
        record = _effect_record(effect_name, "observe.end", response)
        evidence.append(record)
        if not response.ok or response.exit_code != 0:
            errors.append(record)
    return errors


def _call_plugin(
    command: Sequence[str],
    *,
    method: str,
    params: dict[str, JsonValue],
    workspace: TactusWorkspace,
    environment: dict[str, str],
    plugin_executor: PluginExecutor | None,
) -> PluginResponse:
    if plugin_executor is None:
        return invoke_plugin(
            command,
            method=method,
            params=params,
            cwd=workspace.root,
            environment=environment,
        )
    return invoke_plugin(
        command,
        method=method,
        params=params,
        cwd=workspace.root,
        environment=environment,
        executor=plugin_executor,
    )


def _effect_record(
    effect_name: str,
    method: str,
    response: PluginResponse,
) -> dict[str, JsonValue]:
    return {
        "effect": effect_name,
        "method": method,
        "request_id": response.request_id,
        "ok": response.ok and response.exit_code == 0,
        "value": response.value,
        "error": response.error,
        "events": list(response.events),
        "exit_code": response.exit_code,
    }


def _doctor(arguments: argparse.Namespace, *, locator: ToolLocator | None) -> int:
    candidate = workspace_paths(arguments.root)
    workspace_errors: list[str] = []
    try:
        open_workspace(arguments.root)
    except TactusRuntimeError as exc:
        workspace_errors.append(str(exc))
    tools = tool_diagnostics(locator)
    value = {
        "workspace": str(candidate.root),
        "initialized": not workspace_errors,
        "workspace_errors": workspace_errors,
        "tools": [
            {"name": tool.name, "available": tool.available, "path": tool.path}
            for tool in tools
        ],
    }
    if arguments.json:
        _print_json(value)
    else:
        print(
            f"workspace {'ok' if not workspace_errors else 'missing'}: {candidate.root}"
        )
        for error in workspace_errors:
            print(f"  {error}")
        for tool in tools:
            print(f"{tool.name:<8} {tool.path or 'MISSING'}")
    return 0 if not workspace_errors and all(tool.available for tool in tools) else 2


def _smoke(
    workspace: TactusWorkspace,
    selected: Sequence[str],
    *,
    live: bool,
    as_json: bool,
    plugin_executor: PluginExecutor | None,
) -> int:
    runtime = write_runtime_config(workspace)
    config = load_config(workspace)
    available = plugin_commands(config)
    plugins = _select_plugins(available, selected)
    environment = runtime_environment(workspace)
    environment["PATH"] = effective_path()
    results: list[dict[str, object]] = []
    failed = False
    for kind, name, command in plugins:
        params = _smoke_params(runtime, workspace, kind, name, live=live)
        try:
            if plugin_executor is None:
                response = invoke_plugin(
                    command,
                    method="smoke",
                    params=params,
                    cwd=workspace.root,
                    environment=environment,
                )
            else:
                response = invoke_plugin(
                    command,
                    method="smoke",
                    params=params,
                    cwd=workspace.root,
                    environment=environment,
                    executor=plugin_executor,
                )
            ok = response.ok and response.exit_code == 0
            failed = failed or not ok
            results.append(
                {
                    "kind": kind,
                    "name": name,
                    "ok": ok,
                    "value": response.value,
                    "error": response.error,
                    "events": list(response.events),
                    "exit_code": response.exit_code,
                }
            )
        except TactusRuntimeError as exc:
            failed = True
            results.append(
                {
                    "kind": kind,
                    "name": name,
                    "ok": False,
                    "error": str(exc),
                }
            )
    value = {"api": "agenstro.plugin/v1", "live": live, "plugins": results}
    if as_json:
        _print_json(value)
    else:
        for result in results:
            state = "ok" if result["ok"] else "failed"
            print(f"{result['kind']} {result['name']}: {state}")
            if not result["ok"]:
                print(f"  {result.get('error')}", file=sys.stderr)
    return 1 if failed else 0


def _smoke_params(
    runtime: dict[str, JsonValue],
    workspace: TactusWorkspace,
    kind: str,
    name: str,
    *,
    live: bool,
) -> dict[str, JsonValue]:
    registry_name = "providers" if kind == "provider" else "effects"
    registry = runtime.get(registry_name)
    if not isinstance(registry, dict):
        raise ConfigurationError(f"runtime {registry_name} registry is invalid")
    configured = registry.get(name)
    if not isinstance(configured, dict):
        raise ConfigurationError(f"runtime {kind} {name!r} is invalid")
    options = configured.get("options")
    if not isinstance(options, dict):
        raise ConfigurationError(f"runtime {kind} {name!r} options are invalid")
    params: dict[str, JsonValue] = {
        "workspace": str(workspace.root),
        "live": live,
        "options": options,
    }
    if kind == "provider":
        params["model"] = configured.get("model")
        params["effort"] = configured.get("effort")
    return params


def _selected_scripts(
    workspace: TactusWorkspace,
    selected: list[Path],
    *,
    entries_only: bool,
) -> tuple[Path, ...]:
    if selected:
        return explicit_scripts(workspace, selected)
    scripts = discover_scripts(workspace)
    return tuple(
        script.path for script in scripts if script.runnable or not entries_only
    )


def _generation_prompt(instructions: str, goal: str) -> str:
    return (
        f"{instructions.rstrip()}\n\n"
        "# Generation task\n\n"
        f"Goal: {goal}\n\n"
        "Work directly in the workspace. Create the goal as one or more ordinary "
        "Haskell programs under `.tactus/scripts/`, using increasing "
        "`NNN_slug.hs` or `NNN_slug.lhs` entry names. You may create arbitrary "
        "Haskell helper modules there. Do not run the scripts; Tactus will list "
        "the resulting files after you finish.\n"
    )


def _required_runtime_text(value: object, location: str) -> str:
    if not isinstance(value, str) or not value:
        raise ConfigurationError(f"runtime {location} must be non-empty text")
    return value


def _required_runtime_command(value: object, provider: str) -> tuple[str, ...]:
    if not isinstance(value, list) or not value:
        raise ConfigurationError(
            f"provider {provider!r} command must be a non-empty argv array"
        )
    command: list[str] = []
    for part in cast(list[object], value):
        if not isinstance(part, str) or not part:
            raise ConfigurationError(
                f"provider {provider!r} command must be a non-empty argv array"
            )
        command.append(part)
    return tuple(command)


def _script_value(script: ScriptInfo) -> dict[str, object]:
    # Kept local to the CLI so JSON/text presentation does not constrain the model.
    return {
        "path": script.relative_path,
        "kind": script.kind,
        "order": script.order,
        "runnable": script.runnable,
        "warning": script.warning,
    }


def _print_script_rows(scripts: Sequence[ScriptInfo]) -> None:
    for script in scripts:
        order = "---" if script.order is None else f"{script.order:03d}"
        print(f"{order} {script.kind:<6} {script.relative_path}")
        if script.warning:
            print(
                f"tactus: warning: {script.relative_path}: {script.warning}",
                file=sys.stderr,
            )


def _select_plugins(
    available: tuple[tuple[str, str, tuple[str, ...]], ...],
    selected: Sequence[str],
) -> tuple[tuple[str, str, tuple[str, ...]], ...]:
    if not selected:
        return available
    chosen: list[tuple[str, str, tuple[str, ...]]] = []
    for selector in selected:
        matches = [
            plugin
            for plugin in available
            if selector in {plugin[1], f"{plugin[0]}:{plugin[1]}"}
        ]
        if not matches:
            raise ConfigurationError(f"unknown plugin {selector!r}")
        for match in matches:
            if match not in chosen:
                chosen.append(match)
    return tuple(chosen)


def _split_script_arguments(values: list[str]) -> tuple[list[str], list[str]]:
    try:
        boundary = values.index("--")
    except ValueError:
        return values, []
    return values[:boundary], values[boundary + 1 :]


def _root_argument(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--root", type=Path, default=Path.cwd())


def _print_json(value: object) -> None:
    print(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True))


__all__ = ["build_parser", "main"]
