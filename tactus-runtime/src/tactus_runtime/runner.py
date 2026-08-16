"""Direct Haskell toolchain execution for Tactus scripts."""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path

from .errors import ToolError, WorkspaceError
from .workspace import (
    TactusWorkspace,
    runtime_environment,
    write_runtime_config,
)

ToolLocator = Callable[[str], str | None]
CommandExecutor = Callable[..., subprocess.CompletedProcess[object]]


@dataclass(frozen=True, slots=True)
class ToolDiagnostic:
    """Availability of one command-line dependency."""

    name: str
    path: str | None

    @property
    def available(self) -> bool:
        """Return whether the command was resolved."""
        return self.path is not None


def check_scripts(
    workspace: TactusWorkspace,
    scripts: Sequence[Path],
    *,
    keep_going: bool = False,
    locator: ToolLocator | None = None,
    executor: CommandExecutor = subprocess.run,
) -> int:
    """Build Clef and type-check Haskell files without executing them."""
    if not scripts:
        raise WorkspaceError("no Haskell scripts were selected for checking")
    write_runtime_config(workspace)
    locate = locate_tool if locator is None else locator
    cabal = _required_tool("cabal", locate)
    _required_tool("ghc", locate)
    environment = _child_environment(workspace)
    project_directory = str(workspace.control)
    built = _run(
        [cabal, "build", "--project-dir", project_directory, "lib:clef-sdk"],
        workspace=workspace,
        environment=environment,
        executor=executor,
    )
    if built != 0:
        return built

    first_failure = 0
    include = f"-i{workspace.scripts_path}"
    for script in scripts:
        status = _run(
            [
                cabal,
                "exec",
                "--project-dir",
                project_directory,
                "--",
                "ghc",
                "-fno-code",
                "-package",
                "clef-sdk",
                include,
                str(script),
            ],
            workspace=workspace,
            environment=environment,
            executor=executor,
        )
        if status != 0:
            if first_failure == 0:
                first_failure = status
            if not keep_going:
                break
    return first_failure


def run_scripts(
    workspace: TactusWorkspace,
    scripts: Sequence[Path],
    script_arguments: Sequence[str] = (),
    *,
    keep_going: bool = False,
    locator: ToolLocator | None = None,
    executor: CommandExecutor = subprocess.run,
) -> int:
    """Run ordinary Haskell programs in order with inherited process streams."""
    if not scripts:
        raise WorkspaceError("no numbered Haskell entry scripts were found")
    write_runtime_config(workspace)
    locate = locate_tool if locator is None else locator
    cabal = _required_tool("cabal", locate)
    _required_tool("runghc", locate)
    environment = _child_environment(workspace)
    project_directory = str(workspace.control)
    built = _run(
        [cabal, "build", "--project-dir", project_directory, "lib:clef-sdk"],
        workspace=workspace,
        environment=environment,
        executor=executor,
    )
    if built != 0:
        return built

    first_failure = 0
    include = f"-i{workspace.scripts_path}"
    for script in scripts:
        status = _run(
            [
                cabal,
                "exec",
                "--project-dir",
                project_directory,
                "--",
                "runghc",
                "--ghc-arg=-package=clef-sdk",
                f"--ghc-arg={include}",
                str(script),
                *script_arguments,
            ],
            workspace=workspace,
            environment=environment,
            executor=executor,
        )
        if status != 0:
            if first_failure == 0:
                first_failure = status
            if not keep_going:
                break
    return first_failure


def tool_diagnostics(locator: ToolLocator | None = None) -> tuple[ToolDiagnostic, ...]:
    """Resolve every tool required by check and run without starting it."""
    locate = locate_tool if locator is None else locator
    return tuple(
        ToolDiagnostic(name, locate(name)) for name in ("cabal", "ghc", "runghc")
    )


def locate_tool(name: str) -> str | None:
    """Find a tool using the process PATH plus refreshed Windows PATH values."""
    return shutil.which(name, path=effective_path())


def effective_path() -> str:
    """Return PATH augmented with user/machine values on long-lived Windows hosts."""
    values = [os.environ.get("PATH", "")]
    if os.name == "nt":
        values.extend(_windows_paths())
    segments: list[str] = []
    seen: set[str] = set()
    for value in values:
        for segment in value.split(os.pathsep):
            expanded = os.path.expandvars(segment.strip())
            key = expanded.casefold() if os.name == "nt" else expanded
            if expanded and key not in seen:
                seen.add(key)
                segments.append(expanded)
    return os.pathsep.join(segments)


def _windows_paths() -> tuple[str, ...]:
    try:
        import winreg
    except ImportError:
        return ()
    locations = (
        (winreg.HKEY_CURRENT_USER, r"Environment"),
        (
            winreg.HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
    )
    values: list[str] = []
    for hive, key_name in locations:
        try:
            with winreg.OpenKey(hive, key_name) as key:
                value, _ = winreg.QueryValueEx(key, "Path")
        except OSError:
            continue
        if isinstance(value, str):
            values.append(value)
    return tuple(values)


def _child_environment(workspace: TactusWorkspace) -> dict[str, str]:
    environment = runtime_environment(workspace)
    environment["PATH"] = effective_path()
    return environment


def _required_tool(name: str, locator: ToolLocator) -> str:
    path = locator(name)
    if path is None:
        raise ToolError(
            f"required tool `{name}` was not found; install GHCup or refresh PATH, "
            "then run `tactus doctor`"
        )
    return path


def _run(
    command: list[str],
    *,
    workspace: TactusWorkspace,
    environment: dict[str, str],
    executor: CommandExecutor,
) -> int:
    try:
        completed = executor(
            command,
            cwd=workspace.root,
            env=environment,
            check=False,
        )
    except OSError as exc:
        raise ToolError(f"cannot start {command[0]}: {exc}") from exc
    return int(completed.returncode)


__all__ = [
    "CommandExecutor",
    "ToolDiagnostic",
    "ToolLocator",
    "check_scripts",
    "effective_path",
    "locate_tool",
    "run_scripts",
    "tool_diagnostics",
]
