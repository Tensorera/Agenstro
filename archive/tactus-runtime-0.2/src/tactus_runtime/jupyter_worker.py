"""Frozen 0.2 Tactus worker backed by ``jupyter_client``."""

from __future__ import annotations

import sys

from .jupyter_bridge import JupyterExecutionEngine
from .worker import FramedWorkerServer


def main() -> int:
    """Run one fresh-kernel worker over the bounded frame contract."""
    return FramedWorkerServer(JupyterExecutionEngine()).run(
        sys.stdin.buffer,
        sys.stdout.buffer,
        sys.stderr.buffer,
    )


if __name__ == "__main__":
    raise SystemExit(main())


__all__ = ["main"]
