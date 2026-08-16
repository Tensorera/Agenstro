"""Helper functions imported by the example main stage."""

from __future__ import annotations

from pathlib import Path


def describe_files(material_directory: Path) -> list[str]:
    lines: list[str] = []
    for path in sorted(material_directory.iterdir()):
        if path.is_file():
            lines.append(f"- {path.name}: {path.stat().st_size} bytes")
    return lines or ["- No input files were collected."]
