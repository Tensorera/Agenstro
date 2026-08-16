"""Reference one-shot adapters for Codex, Claude Code, and OpenCode."""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import TextIO, cast

from . import __version__
from .plugin_protocol import (
    API_VERSION,
    EventWriter,
    JsonObject,
    PluginError,
    PluginRequest,
    configure_utf8_standard_stream,
    run_plugin,
    split_jsonl,
)

_OPENCODE_WARNING = (
    "OpenCode --auto only approves ask decisions and does not override explicit "
    "deny or managed configuration; full bypass cannot be guaranteed."
)


@dataclass(frozen=True, slots=True)
class ProviderSpec:
    """Static CLI and permission properties for one provider adapter."""

    name: str
    executable: str
    aliases: tuple[str, ...]
    full_bypass: bool
    warning: str | None = None


@dataclass(frozen=True, slots=True)
class InvocationOptions:
    command_prefix: tuple[str, ...]
    timeout_seconds: float | None
    extra_args: tuple[str, ...]
    extra_env: dict[str, str]
    open_options: JsonObject


_PROVIDERS = {
    "codex": ProviderSpec("codex", "codex", (), True),
    "claude-code": ProviderSpec("claude-code", "claude", ("claude",), True),
    "opencode": ProviderSpec(
        "opencode",
        "opencode",
        (),
        False,
        _OPENCODE_WARNING,
    ),
}
_ALIASES = {alias: spec.name for spec in _PROVIDERS.values() for alias in spec.aliases}


def build_parser() -> argparse.ArgumentParser:
    """Build the standalone provider host parser."""
    parser = argparse.ArgumentParser(
        prog="tactus-provider-host",
        description="Run one Tactus provider plugin request from stdin.",
    )
    parser.add_argument(
        "provider",
        choices=sorted([*_PROVIDERS, *_ALIASES]),
        help="Canonical provider name (the 'claude' alias is also accepted).",
    )
    return parser


def main(
    argv: Sequence[str] | None = None,
    *,
    stdin: TextIO | None = None,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
) -> int:
    """Run one provider request and return a process exit code."""
    input_stream = configure_utf8_standard_stream(sys.stdin) if stdin is None else stdin
    output_stream = (
        configure_utf8_standard_stream(sys.stdout) if stdout is None else stdout
    )
    arguments = build_parser().parse_args(argv)
    provider_argument = str(arguments.provider)
    provider_name = _ALIASES.get(provider_argument, provider_argument)
    spec = _PROVIDERS[provider_name]
    error_stream = sys.stderr if stderr is None else stderr
    return run_plugin(
        lambda request, writer: handle_request(spec, request, writer, error_stream),
        stdin=input_stream,
        stdout=output_stream,
        stderr=error_stream,
    )


def handle_request(
    spec: ProviderSpec,
    request: PluginRequest,
    writer: EventWriter,
    stderr: TextIO,
) -> object:
    """Dispatch one validated protocol request for ``spec``."""
    if request.method == "describe":
        return _describe(spec)
    if request.method == "smoke":
        return _smoke(spec, request.params, writer, stderr)
    if request.method == "invoke":
        return _invoke(spec, request.params, writer, stderr)
    raise PluginError(
        "method_not_found",
        f"provider {spec.name!r} does not implement {request.method!r}",
        details={"methods": ["describe", "smoke", "invoke"]},
    )


def _describe(spec: ProviderSpec) -> JsonObject:
    operations = ["describe", "smoke", "invoke"]
    value: JsonObject = {
        "api": API_VERSION,
        "kind": "provider",
        "name": spec.name,
        "implementation_version": __version__,
        "aliases": list(spec.aliases),
        "executable": spec.executable,
        "methods": operations,
        "operations": operations,
        "full_bypass": spec.full_bypass,
        "reasoning_parameter": "variant" if spec.name == "opencode" else "effort",
        "options_schema": {
            "type": "object",
            "additionalProperties": True,
            "properties": {
                "command_prefix": {"type": "array", "items": {"type": "string"}},
                "timeout_seconds": {"type": "number", "exclusiveMinimum": 0},
                "extra_args": {"type": "array", "items": {"type": "string"}},
                "extra_env": {
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                },
                "auth_status": {"type": "boolean"},
            },
        },
    }
    if spec.warning is not None:
        value["warning"] = spec.warning
    return value


def _smoke(
    spec: ProviderSpec,
    params: JsonObject,
    writer: EventWriter,
    stderr: TextIO,
) -> JsonObject:
    options = _read_invocation_options(params, default_timeout=20.0)
    environment = _environment(spec, options.extra_env)
    executable = _provider_executable(spec, options, environment)
    workspace = _optional_workspace(params)
    version_command = [*options.command_prefix, executable, "--version"]
    version = _run_simple_command(
        spec,
        version_command,
        environment=environment,
        cwd=workspace,
        timeout_seconds=options.timeout_seconds,
        stderr=stderr,
    )
    auth_status: str | None = None
    if _bool_option(params, options.open_options, "auth_status", default=False):
        auth_status = _run_simple_command(
            spec,
            [*options.command_prefix, *_auth_command(spec, executable)],
            environment=environment,
            cwd=workspace,
            timeout_seconds=options.timeout_seconds,
            stderr=stderr,
        )

    live = _bool_option(params, options.open_options, "live", default=False)
    if not live:
        value: JsonObject = {
            "provider": spec.name,
            "text": version.strip(),
            "version": version.strip(),
            "live": False,
            "full_bypass": spec.full_bypass,
        }
        if auth_status is not None:
            value["auth_status"] = auth_status.strip()
        if spec.warning is not None:
            value["warning"] = spec.warning
        return value

    live_params = dict(params)
    live_params["prompt"] = "Reply exactly TACTUS_OK. Do not use tools."
    if "workspace" not in live_params:
        live_params["workspace"] = str(Path.cwd().resolve())
    if spec.name == "claude-code":
        live_params["extra_args"] = [*options.extra_args, "--tools", ""]
    result = _invoke(spec, live_params, writer, stderr)
    text = result["text"]
    if not isinstance(text, str) or "TACTUS_OK" not in text:
        raise PluginError(
            "smoke_mismatch",
            f"{spec.name} live smoke did not return TACTUS_OK",
            details={"provider": spec.name, "text": text},
        )
    result["version"] = version.strip()
    result["live"] = True
    if auth_status is not None:
        result["auth_status"] = auth_status.strip()
    return result


def _invoke(
    spec: ProviderSpec,
    params: JsonObject,
    writer: EventWriter,
    stderr: TextIO,
) -> JsonObject:
    prompt = _required_string(params, "prompt")
    workspace = _required_workspace(params)
    # A normal provider invocation has no framework-imposed deadline. Callers
    # can opt into one with options.timeout_seconds.
    options = _read_invocation_options(params, default_timeout=None)
    model = _optional_string(params, "model") or _optional_string(
        options.open_options, "model"
    )
    effort = _optional_string(params, "effort") or _optional_string(
        options.open_options, "effort"
    )
    if spec.name == "opencode":
        effort = (
            _optional_string(params, "variant")
            or _optional_string(options.open_options, "variant")
            or effort
        )
    environment = _environment(spec, options.extra_env)
    executable = _provider_executable(spec, options, environment)

    if spec.name == "codex":
        with tempfile.TemporaryDirectory(prefix="tactus-codex-") as temporary:
            last_message = Path(temporary) / "last-message.txt"
            argv = _codex_argv(
                spec,
                executable,
                workspace,
                model,
                effort,
                options,
                last_message,
            )
            completed = _run_provider(
                spec,
                argv,
                prompt,
                environment,
                workspace,
                options.timeout_seconds,
                stderr,
            )
            event_text = _emit_provider_output(spec, completed.stdout, writer)
            file_text = _read_last_message(last_message)
            text = file_text if file_text is not None else event_text
            _raise_for_exit(spec, completed.returncode, text)
            return _provider_value(spec, text)

    argv = (
        _claude_argv(spec, executable, workspace, model, effort, options)
        if spec.name == "claude-code"
        else _opencode_argv(spec, executable, workspace, model, effort, options)
    )
    completed = _run_provider(
        spec,
        argv,
        prompt,
        environment,
        workspace,
        options.timeout_seconds,
        stderr,
    )
    text = _emit_provider_output(spec, completed.stdout, writer)
    _raise_for_exit(spec, completed.returncode, text)
    return _provider_value(spec, text)


def _codex_argv(
    spec: ProviderSpec,
    executable: str,
    workspace: Path,
    model: str | None,
    effort: str | None,
    options: InvocationOptions,
    last_message: Path,
) -> list[str]:
    argv = [
        *options.command_prefix,
        executable,
        "exec",
        "--dangerously-bypass-approvals-and-sandbox",
        "--json",
        "-C",
        str(workspace),
        "--skip-git-repo-check",
        "--ephemeral",
    ]
    if model is not None:
        argv.extend(["--model", model])
    if effort is not None:
        argv.extend(["-c", f"model_reasoning_effort={json.dumps(effort)}"])
    argv.extend(["-o", str(last_message), *options.extra_args, "-"])
    return argv


def _claude_argv(
    spec: ProviderSpec,
    executable: str,
    workspace: Path,
    model: str | None,
    effort: str | None,
    options: InvocationOptions,
) -> list[str]:
    del workspace
    argv = [
        *options.command_prefix,
        executable,
        "-p",
        "--dangerously-skip-permissions",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
    ]
    if model is not None:
        argv.extend(["--model", model])
    if effort is not None:
        argv.extend(["--effort", effort])
    argv.extend(options.extra_args)
    return argv


def _opencode_argv(
    spec: ProviderSpec,
    executable: str,
    workspace: Path,
    model: str | None,
    effort: str | None,
    options: InvocationOptions,
) -> list[str]:
    argv = [
        *options.command_prefix,
        executable,
        "run",
        "--auto",
        "--format",
        "json",
        "--dir",
        str(workspace),
    ]
    if model is not None:
        argv.extend(["--model", model])
    if effort is not None:
        argv.extend(["--variant", effort])
    argv.extend(options.extra_args)
    return argv


def _run_provider(
    spec: ProviderSpec,
    argv: list[str],
    prompt: str,
    environment: Mapping[str, str],
    workspace: Path,
    timeout_seconds: float | None,
    stderr: TextIO,
) -> subprocess.CompletedProcess[str]:
    try:
        completed = subprocess.run(
            argv,
            cwd=workspace,
            env=environment,
            input=prompt,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout_seconds,
            check=False,
        )
    except FileNotFoundError as exc:
        raise PluginError(
            "provider_not_found",
            f"could not start {spec.executable!r}",
            details={"provider": spec.name, "executable": spec.executable},
        ) from exc
    except subprocess.TimeoutExpired as exc:
        raise PluginError(
            "outcome_unknown",
            f"{spec.name} may have completed externally before its configured timeout",
            details={
                "provider": spec.name,
                "cause": "timeout",
                "timeout_seconds": timeout_seconds,
            },
        ) from exc
    _forward_diagnostics(spec, completed.stderr, stderr)
    return completed


def _run_simple_command(
    spec: ProviderSpec,
    argv: list[str],
    *,
    environment: Mapping[str, str],
    cwd: Path | None,
    timeout_seconds: float | None,
    stderr: TextIO,
) -> str:
    try:
        completed = subprocess.run(
            argv,
            cwd=cwd,
            env=environment,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout_seconds,
            check=False,
        )
    except FileNotFoundError as exc:
        raise PluginError(
            "provider_not_found",
            f"could not start {spec.executable!r}",
            details={"provider": spec.name, "executable": spec.executable},
        ) from exc
    except subprocess.TimeoutExpired as exc:
        raise PluginError(
            "provider_timeout",
            f"{spec.name} health command exceeded its configured timeout",
            details={"provider": spec.name, "timeout_seconds": timeout_seconds},
        ) from exc
    _forward_diagnostics(spec, completed.stderr, stderr)
    if completed.returncode != 0:
        raise PluginError(
            "provider_health_failed",
            f"{spec.name} health command exited with {completed.returncode}",
            details={"provider": spec.name, "exit_code": completed.returncode},
        )
    return completed.stdout


def _emit_provider_output(
    spec: ProviderSpec,
    stdout: str,
    writer: EventWriter,
) -> str:
    fragments: list[str] = []
    final_text: str | None = None
    plain_lines: list[str] = []
    for line in split_jsonl(stdout):
        if not line.strip():
            continue
        try:
            raw: object = json.loads(line)
        except json.JSONDecodeError:
            plain_lines.append(line)
            writer.event("provider.output", provider=spec.name, text=line)
            continue
        writer.event("provider.raw", provider=spec.name, raw=raw)
        event_fragments, event_final = _event_text(spec, raw)
        fragments.extend(event_fragments)
        if event_final is not None:
            final_text = event_final
    if final_text is not None:
        return final_text
    if fragments:
        return "".join(fragments)
    return "\n".join(plain_lines)


def _event_text(spec: ProviderSpec, raw: object) -> tuple[list[str], str | None]:
    event = _string_keyed_object(raw)
    if event is None:
        return [], None
    event_type = event.get("type")
    if spec.name == "codex" and event_type == "item.completed":
        item = _string_keyed_object(event.get("item"))
        if item is not None and item.get("type") == "agent_message":
            text = item.get("text")
            if isinstance(text, str):
                return [], text
    if spec.name == "claude-code":
        if event_type == "result":
            result = event.get("result")
            if isinstance(result, str):
                return [], result
        if event_type == "assistant":
            return _claude_message_text(event), None
    if spec.name == "opencode" and event_type == "text":
        part = _string_keyed_object(event.get("part"))
        if part is not None:
            text = part.get("text")
            if isinstance(text, str):
                return [text], None
        text = event.get("text")
        if isinstance(text, str):
            return [text], None
    return [], None


def _claude_message_text(raw: Mapping[str, object]) -> list[str]:
    message = raw.get("message")
    message_object = _string_keyed_object(message)
    if message_object is None:
        return []
    content = message_object.get("content")
    if not isinstance(content, list):
        return []
    result: list[str] = []
    for raw_part in cast(list[object], content):
        part = _string_keyed_object(raw_part)
        if part is not None and part.get("type") == "text":
            text = part.get("text")
            if isinstance(text, str):
                result.append(text)
    return result


def _provider_value(spec: ProviderSpec, text: str) -> JsonObject:
    value: JsonObject = {
        "provider": spec.name,
        "text": text,
        "exit_code": 0,
        "full_bypass": spec.full_bypass,
    }
    if spec.warning is not None:
        value["warning"] = spec.warning
    return value


def _raise_for_exit(spec: ProviderSpec, exit_code: int, text: str) -> None:
    if exit_code == 0:
        return
    details: JsonObject = {
        "provider": spec.name,
        "cause": "provider_exit",
        "exit_code": exit_code,
        "text": text,
        "full_bypass": spec.full_bypass,
    }
    if spec.warning is not None:
        details["warning"] = spec.warning
    raise PluginError(
        "outcome_unknown",
        f"{spec.name} exited without proving whether the external request completed",
        details=details,
    )


def _read_last_message(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None


def _forward_diagnostics(spec: ProviderSpec, value: str, stderr: TextIO) -> None:
    if not value:
        return
    for line in value.splitlines():
        print(f"[{spec.name}] {line}", file=stderr)


def _environment(spec: ProviderSpec, extra: Mapping[str, str]) -> dict[str, str]:
    environment = dict(os.environ)
    environment.update(extra)
    if spec.name != "opencode":
        return environment
    inline: JsonObject = {}
    existing = environment.get("OPENCODE_CONFIG_CONTENT")
    if existing:
        try:
            decoded: object = json.loads(existing)
        except json.JSONDecodeError:
            decoded = None
        decoded_object = _string_keyed_object(decoded)
        if decoded_object is not None:
            inline.update(decoded_object)
    inline["permission"] = "allow"
    environment["OPENCODE_CONFIG_CONTENT"] = json.dumps(
        inline,
        ensure_ascii=False,
        separators=(",", ":"),
    )
    return environment


def _provider_executable(
    spec: ProviderSpec,
    options: InvocationOptions,
    environment: Mapping[str, str],
) -> str:
    """Resolve Windows npm ``.cmd`` shims without introducing a shell."""
    if options.command_prefix:
        return spec.executable
    return (
        shutil.which(spec.executable, path=environment.get("PATH")) or spec.executable
    )


def _auth_command(spec: ProviderSpec, executable: str) -> list[str]:
    if spec.name == "codex":
        return [executable, "login", "status"]
    if spec.name == "claude-code":
        return [executable, "auth", "status", "--json"]
    return [executable, "auth", "list"]


def _read_invocation_options(
    params: Mapping[str, object],
    *,
    default_timeout: float | None,
) -> InvocationOptions:
    raw_options = _string_keyed_object(params.get("options", {}))
    if raw_options is None:
        raise PluginError("invalid_params", "options must be a JSON object")
    open_options: JsonObject = dict(raw_options)
    raw_prefix = open_options.get("command_prefix", [])
    if not isinstance(raw_prefix, list):
        raise PluginError(
            "invalid_params",
            "options.command_prefix must be an array of non-empty strings",
        )
    prefix_values = cast(list[object], raw_prefix)
    if not all(isinstance(part, str) and part for part in prefix_values):
        raise PluginError(
            "invalid_params",
            "options.command_prefix must be an array of non-empty strings",
        )
    command_prefix = [cast(str, part) for part in prefix_values]
    raw_timeout = open_options.get("timeout_seconds", default_timeout)
    if raw_timeout is None:
        timeout: float | None = None
    elif isinstance(raw_timeout, bool) or not isinstance(raw_timeout, (int, float)):
        raise PluginError("invalid_params", "options.timeout_seconds must be a number")
    else:
        timeout = float(raw_timeout)
        if not math.isfinite(timeout) or timeout <= 0:
            raise PluginError(
                "invalid_params", "options.timeout_seconds must be positive"
            )
    raw_extra_args = params.get(
        "extra_args",
        open_options.get("extra_args", []),
    )
    extra_args = _string_list_value(raw_extra_args, "extra_args")
    raw_env = _string_keyed_object(
        params.get("extra_env", open_options.get("extra_env", {}))
    )
    if raw_env is None or not all(isinstance(value, str) for value in raw_env.values()):
        raise PluginError("invalid_params", "extra_env must map strings to strings")
    extra_env = {key: cast(str, value) for key, value in raw_env.items()}
    return InvocationOptions(
        command_prefix=tuple(command_prefix),
        timeout_seconds=timeout,
        extra_args=tuple(extra_args),
        extra_env=extra_env,
        open_options=open_options,
    )


def _required_workspace(params: Mapping[str, object]) -> Path:
    workspace = _optional_workspace(params)
    if workspace is None:
        raise PluginError("invalid_params", "workspace must be a non-empty string")
    return workspace


def _optional_workspace(params: Mapping[str, object]) -> Path | None:
    value = params.get("workspace")
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise PluginError("invalid_params", "workspace must be a non-empty string")
    path = Path(value).resolve()
    if not path.is_dir():
        raise PluginError(
            "invalid_params",
            "workspace must name an existing directory",
            details={"workspace": str(path)},
        )
    return path


def _required_string(params: Mapping[str, object], key: str) -> str:
    value = _optional_string(params, key)
    if value is None:
        raise PluginError("invalid_params", f"{key} must be a non-empty string")
    return value


def _optional_string(params: Mapping[str, object], key: str) -> str | None:
    value = params.get(key)
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise PluginError("invalid_params", f"{key} must be a non-empty string")
    return value


def _string_list_value(value: object, key: str) -> list[str]:
    if not isinstance(value, list):
        raise PluginError("invalid_params", f"{key} must be an array of strings")
    items = cast(list[object], value)
    if not all(isinstance(item, str) for item in items):
        raise PluginError("invalid_params", f"{key} must be an array of strings")
    return [cast(str, item) for item in items]


def _bool_option(
    params: Mapping[str, object],
    options: Mapping[str, object],
    key: str,
    *,
    default: bool,
) -> bool:
    value = params.get(key, options.get(key, default))
    if not isinstance(value, bool):
        raise PluginError("invalid_params", f"{key} must be a boolean")
    return value


def _string_keyed_object(value: object) -> dict[str, object] | None:
    if not isinstance(value, dict):
        return None
    raw = cast(dict[object, object], value)
    converted: dict[str, object] = {}
    for key, item in raw.items():
        if not isinstance(key, str):
            return None
        converted[key] = item
    return converted


if __name__ == "__main__":  # pragma: no cover - exercised through subprocess tests
    raise SystemExit(main())


__all__ = ["ProviderSpec", "build_parser", "handle_request", "main"]
