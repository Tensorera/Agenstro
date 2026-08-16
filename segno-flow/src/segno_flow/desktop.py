"""``segno-flow-ui`` launcher for Motivo Studio's scheduler surface."""

from __future__ import annotations

import os
import sys
from collections.abc import Callable, Sequence

MOTIVO_STUDIO_ENTRYPOINT = "motivo-studio"
ExecFunction = Callable[[str, list[str]], object]


def build_launch_command(arguments: Sequence[str]) -> list[str]:
    """Build the fixed Motivo surface request without a shell."""

    return [MOTIVO_STUDIO_ENTRYPOINT, "--surface", "scheduler", *arguments]


def main(
    argv: Sequence[str] | None = None,
    *,
    executor: ExecFunction = os.execvp,
) -> int:
    """Replace this facade with Motivo; never own a GUI child process."""

    command = build_launch_command(sys.argv[1:] if argv is None else argv)
    try:
        executor(command[0], command)
    except OSError as error:
        print(
            f"segno-flow-ui could not launch Motivo Studio's Scheduler surface: {error}",
            file=sys.stderr,
        )
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
