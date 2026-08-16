"""Bounded deterministic task-package authoring without script execution."""

from __future__ import annotations

import json
import os
import stat
import tempfile
import unicodedata
import zipfile
from dataclasses import dataclass
from pathlib import Path

from segno_flow.manifest import MANIFEST_NAME, ManifestError, PackageManifest, load_manifest

_WINDOWS_DEVICE_NAMES = {
    "CON",
    "PRN",
    "AUX",
    "NUL",
    *(f"COM{number}" for number in range(1, 10)),
    *(f"LPT{number}" for number in range(1, 10)),
}

_HARD_MAX_ARCHIVE_BYTES = 64 * 1024 * 1024
_HARD_MAX_ENTRIES = 1_000
_HARD_MAX_FILE_BYTES = 64 * 1024 * 1024
_HARD_MAX_EXPANDED_BYTES = 256 * 1024 * 1024
_HARD_MAX_COMPRESSION_RATIO = 200
_HARD_MAX_PATH_DEPTH = 32
_HARD_MAX_COMPONENT_BYTES = 255
_HARD_MAX_PATH_BYTES = 4_096


class PackageBuildError(ValueError):
    """A source tree cannot be represented as a safe task package."""


@dataclass(frozen=True, slots=True)
class ArchiveBudget:
    """Hard limits matching the Rust package importer defaults."""

    max_archive_bytes: int = _HARD_MAX_ARCHIVE_BYTES
    max_entries: int = _HARD_MAX_ENTRIES
    max_file_bytes: int = _HARD_MAX_FILE_BYTES
    max_expanded_bytes: int = _HARD_MAX_EXPANDED_BYTES
    max_compression_ratio: int = _HARD_MAX_COMPRESSION_RATIO
    max_path_depth: int = _HARD_MAX_PATH_DEPTH
    max_component_bytes: int = _HARD_MAX_COMPONENT_BYTES
    max_path_bytes: int = _HARD_MAX_PATH_BYTES

    def validate(self) -> None:
        """Reject disabled limits instead of constructing an unbounded builder."""

        values = (
            self.max_archive_bytes,
            self.max_entries,
            self.max_file_bytes,
            self.max_expanded_bytes,
            self.max_compression_ratio,
            self.max_path_depth,
            self.max_component_bytes,
            self.max_path_bytes,
        )
        if any(isinstance(value, bool) or not isinstance(value, int) for value in values):
            raise PackageBuildError("archive budget values must be integers")
        if any(value <= 0 for value in values):
            raise PackageBuildError("archive budget values must be positive")
        if (
            self.max_archive_bytes > _HARD_MAX_ARCHIVE_BYTES
            or self.max_entries > _HARD_MAX_ENTRIES
            or self.max_file_bytes > _HARD_MAX_FILE_BYTES
            or self.max_expanded_bytes > _HARD_MAX_EXPANDED_BYTES
            or self.max_compression_ratio > _HARD_MAX_COMPRESSION_RATIO
            or self.max_path_depth > _HARD_MAX_PATH_DEPTH
            or self.max_component_bytes > _HARD_MAX_COMPONENT_BYTES
            or self.max_path_bytes > _HARD_MAX_PATH_BYTES
            or self.max_file_bytes > self.max_expanded_bytes
        ):
            raise PackageBuildError("archive budget exceeds a hard package ceiling")


@dataclass(frozen=True, slots=True)
class PackageBuildResult:
    """Metadata for one atomically published ZIP."""

    path: Path
    manifest: PackageManifest
    entries: int
    expanded_bytes: int
    archive_bytes: int


_DEFAULT_ARCHIVE_BUDGET = ArchiveBudget()


def _portable_path(path: str, budget: ArchiveBudget) -> str:
    if (
        not path
        or len(path.encode("utf-8")) > budget.max_path_bytes
        or path.startswith("/")
        or "\\" in path
        or "\0" in path
        or any(unicodedata.category(character).startswith("C") for character in path)
    ):
        raise PackageBuildError(f"non-portable package path: {path!r}")
    components = path.split("/")
    if len(components) > budget.max_path_depth:
        raise PackageBuildError(f"package path is too deep: {path!r}")
    for component in components:
        stem = component.split(".", 1)[0].upper()
        if (
            not component
            or component in {".", ".."}
            or len(component.encode("utf-8")) > budget.max_component_bytes
            or ":" in component
            or component.endswith((".", " "))
            or stem in _WINDOWS_DEVICE_NAMES
        ):
            raise PackageBuildError(f"non-portable package path: {path!r}")
    return path


def _collision_key(path: str) -> str:
    return unicodedata.normalize("NFC", path).lower()


def _collect_files(
    source_root: Path,
    output: Path,
    budget: ArchiveBudget,
) -> tuple[list[tuple[Path, str, int]], int]:
    pending = [source_root]
    files: list[tuple[Path, str, int]] = []
    seen: set[str] = set()
    total = 0
    visited_entries = 0
    while pending:
        directory = pending.pop()
        try:
            children: list[Path] = []
            for child in directory.iterdir():
                visited_entries += 1
                if visited_entries > budget.max_entries:
                    raise PackageBuildError("package source exceeds the entry budget")
                children.append(child)
            children.sort(key=lambda child: child.name)
        except OSError as error:
            raise PackageBuildError(f"cannot enumerate package source: {directory}") from error
        for child in children:
            try:
                metadata = child.lstat()
            except OSError as error:
                raise PackageBuildError(f"cannot inspect package source: {child}") from error
            if stat.S_ISLNK(metadata.st_mode):
                raise PackageBuildError(f"symbolic links are not allowed: {child}")
            if child == output:
                continue
            relative = _portable_path(child.relative_to(source_root).as_posix(), budget)
            key = _collision_key(relative)
            if key in seen:
                raise PackageBuildError(f"portable package path collision: {relative}")
            seen.add(key)
            if stat.S_ISDIR(metadata.st_mode):
                pending.append(child)
                continue
            if not stat.S_ISREG(metadata.st_mode):
                raise PackageBuildError(f"special files are not allowed: {child}")
            if len(files) >= budget.max_entries:
                raise PackageBuildError("package exceeds the entry budget")
            if metadata.st_size > budget.max_file_bytes:
                raise PackageBuildError(f"package file exceeds the byte budget: {relative}")
            total += metadata.st_size
            if total > budget.max_expanded_bytes:
                raise PackageBuildError("package exceeds the expanded byte budget")
            files.append((child, relative, metadata.st_size))
    files.sort(key=lambda item: item[1])
    return files, total


def _validate_written_archive(path: Path, budget: ArchiveBudget) -> None:
    archive_bytes = path.stat().st_size
    if archive_bytes > budget.max_archive_bytes:
        raise PackageBuildError("package exceeds the compressed byte budget")
    with zipfile.ZipFile(path, "r") as archive:
        infos = archive.infolist()
        if len(infos) > budget.max_entries:
            raise PackageBuildError("package exceeds the entry budget")
        total = 0
        for info in infos:
            total += info.file_size
            if info.file_size > budget.max_file_bytes or total > budget.max_expanded_bytes:
                raise PackageBuildError("written package exceeds the expanded byte budget")
            if info.file_size and (
                info.compress_size == 0
                or info.file_size > info.compress_size * budget.max_compression_ratio
            ):
                raise PackageBuildError(
                    f"package member exceeds the compression ratio: {info.filename}"
                )


def build_task_package(
    source_root: Path,
    output: Path,
    *,
    budget: ArchiveBudget = _DEFAULT_ARCHIVE_BUDGET,
) -> PackageBuildResult:
    """Validate a source tree and atomically build a deterministic ZIP.

    Python files are copied as bytes. They are never imported, compiled, or executed.
    """

    budget.validate()
    source_root = source_root.resolve()
    output = output.resolve()
    if not source_root.is_dir():
        raise PackageBuildError("package source must be an existing directory")
    manifest_path = source_root / MANIFEST_NAME
    try:
        manifest = load_manifest(manifest_path)
    except (OSError, ManifestError) as error:
        raise PackageBuildError(str(error)) from error
    files, expanded_bytes = _collect_files(source_root, output, budget)
    paths = {relative for _, relative, _ in files}
    if MANIFEST_NAME not in paths:
        raise PackageBuildError(f"package root must contain {MANIFEST_NAME}")
    for stage in (manifest.scripts.pre, manifest.scripts.main, manifest.scripts.post):
        if stage not in paths:
            raise PackageBuildError(f"manifest stage file does not exist: {stage}")

    output.parent.mkdir(parents=True, exist_ok=True)
    temporary_path: Path | None = None
    try:
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=f".{output.name}.", suffix=".tmp", dir=output.parent
        )
        os.close(descriptor)
        temporary_path = Path(temporary_name)
        with zipfile.ZipFile(
            temporary_path,
            "w",
            compression=zipfile.ZIP_DEFLATED,
            compresslevel=9,
            strict_timestamps=True,
        ) as archive:
            written_total = 0
            for source, relative, expected_size in files:
                info = zipfile.ZipInfo(relative, date_time=(2020, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = 0o100644 << 16
                if relative == MANIFEST_NAME:
                    payload = (
                        json.dumps(
                            manifest.to_dict(), ensure_ascii=False, indent=2, sort_keys=True
                        ).encode("utf-8")
                        + b"\n"
                    )
                    if (
                        len(payload) > budget.max_file_bytes
                        or written_total + len(payload) > budget.max_expanded_bytes
                    ):
                        raise PackageBuildError("canonical manifest exceeds the byte budget")
                    archive.writestr(info, payload)
                    written_total += len(payload)
                else:
                    with source.open("rb") as source_file, archive.open(info, "w") as target:
                        written = 0
                        while chunk := source_file.read(64 * 1024):
                            written += len(chunk)
                            written_total += len(chunk)
                            if (
                                written > expected_size
                                or written > budget.max_file_bytes
                                or written_total > budget.max_expanded_bytes
                            ):
                                raise PackageBuildError(
                                    f"package source changed or exceeded its budget: {relative}"
                                )
                            target.write(chunk)
                        if written != expected_size:
                            raise PackageBuildError(
                                f"package source changed while reading: {relative}"
                            )
        _validate_written_archive(temporary_path, budget)
        os.replace(temporary_path, output)
        temporary_path = None
    except (OSError, zipfile.BadZipFile) as error:
        raise PackageBuildError("failed to build task package") from error
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)

    return PackageBuildResult(
        path=output,
        manifest=manifest,
        entries=len(files),
        expanded_bytes=expanded_bytes,
        archive_bytes=output.stat().st_size,
    )
