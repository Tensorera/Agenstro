"""Frozen 0.2 ``motivo-studio`` entry."""

from __future__ import annotations

import sys

MIGRATION_MESSAGE = (
    "Motivo Studio is now an Electron application. Install or launch the "
    "motivo-studio desktop bundle; the Python entry no longer hosts pywebview "
    "or owns Tactus runtime state."
)


def main() -> int:
    """Return an explicit migration result without starting a Python GUI."""
    print(MIGRATION_MESSAGE, file=sys.stderr)
    return 2


__all__ = ["MIGRATION_MESSAGE", "main"]
