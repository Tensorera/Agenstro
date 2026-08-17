"""Workfolder preparation and hardened MinerU archive installation."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import stat
import tempfile
import uuid
import zipfile
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path, PurePosixPath
from typing import Any

from .client import MinerUClient, MinerUJob


_ENV_NAME = "Mineru_Api"
_MAX_ARCHIVE_ENTRIES = 50_000
_MAX_UNCOMPRESSED_BYTES = 4 * 1024 * 1024 * 1024


@dataclass(frozen=True, slots=True)
class ExtractionResult:
    manuscript_pdf: Path
    extracted_dir: Path
    full_markdown: Path
    manuscript_sha256: str
    full_markdown_sha256: str
    batch_id: str | None
    reused: bool


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _atomic_json(path: Path, value: Any) -> None:
    payload = (
        json.dumps(
            value,
            ensure_ascii=False,
            sort_keys=True,
            indent=2,
            allow_nan=False,
        )
        + "\n"
    ).encode("utf-8")
    temporary = path.parent / f".{path.name}.tmp-{uuid.uuid4().hex}"
    try:
        with temporary.open("xb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)
    finally:
        if temporary.exists():
            temporary.unlink()


def _parse_env_value(raw: str) -> str:
    value = raw.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
        quote = value[0]
        value = value[1:-1]
        if quote == '"':
            value = bytes(value, "utf-8").decode("unicode_escape")
    return value.strip()


def load_mineru_token(env_file: Path) -> str:
    """Read `Mineru_Api` without importing or mutating process environment."""

    environment_value = os.environ.get(_ENV_NAME)
    if environment_value and environment_value.strip():
        return environment_value.strip()
    try:
        lines = env_file.read_text(encoding="utf-8-sig").splitlines()
    except OSError as error:
        raise RuntimeError(f"cannot read MinerU env file {env_file}: {error}") from error
    found: str | None = None
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("export "):
            stripped = stripped[7:].lstrip()
        if "=" not in stripped:
            continue
        key, raw = stripped.split("=", 1)
        if key.strip() != _ENV_NAME:
            continue
        if found is not None:
            raise ValueError(f"duplicate {_ENV_NAME} entry in {env_file}")
        found = _parse_env_value(raw)
    if not found:
        raise ValueError(
            f"{_ENV_NAME} is missing or empty in environment and {env_file}"
        )
    return found


def prepare_manuscript(source_pdf: Path, workfolder: Path) -> Path:
    """Copy the immutable source into the required `manuscript.pdf` slot."""

    source_pdf = source_pdf.expanduser().resolve(strict=True)
    workfolder = workfolder.expanduser().resolve(strict=False)
    if not source_pdf.is_file() or source_pdf.suffix.casefold() != ".pdf":
        raise ValueError(f"input must be a PDF file: {source_pdf}")
    workfolder.mkdir(parents=True, exist_ok=True)
    target = workfolder / "manuscript.pdf"
    if target.exists():
        if not target.is_file():
            raise FileExistsError(f"manuscript slot is not a file: {target}")
        if _sha256_file(target) != _sha256_file(source_pdf):
            raise FileExistsError(
                "workfolder/manuscript.pdf contains a different document; "
                "use a new workfolder"
            )
        return target
    temporary = workfolder / f".manuscript.pdf.copy-{os.getpid()}"
    try:
        shutil.copy2(source_pdf, temporary)
        if _sha256_file(temporary) != _sha256_file(source_pdf):
            raise OSError("copied manuscript digest does not match source")
        temporary.replace(target)
    finally:
        if temporary.exists():
            temporary.unlink()
    return target


def find_full_markdown(extracted_dir: Path) -> Path:
    extracted_dir = extracted_dir.resolve(strict=True)
    direct = extracted_dir / "full.md"
    if direct.is_file():
        return direct
    candidates = sorted(
        (
            path
            for path in extracted_dir.rglob("full.md")
            if path.is_file()
        ),
        key=lambda path: path.relative_to(extracted_dir).as_posix().casefold(),
    )
    if len(candidates) != 1:
        raise ValueError(
            f"expected exactly one full.md below {extracted_dir}, "
            f"found {len(candidates)}"
        )
    return candidates[0]


def _safe_member_path(root: Path, name: str) -> Path:
    normalized = name.replace("\\", "/")
    pure = PurePosixPath(normalized)
    if (
        pure.is_absolute()
        or not pure.parts
        or any(part in {"", ".", ".."} for part in pure.parts)
        or re.match(r"^[A-Za-z]:", pure.parts[0])
    ):
        raise ValueError(f"unsafe MinerU archive member path: {name!r}")
    target = root.joinpath(*pure.parts).resolve(strict=False)
    if not target.is_relative_to(root.resolve(strict=False)):
        raise ValueError(f"MinerU archive member escapes extraction root: {name!r}")
    return target


def _extract_archive(archive: Path, target: Path) -> None:
    target.mkdir(parents=True, exist_ok=False)
    with zipfile.ZipFile(archive) as bundle:
        infos = bundle.infolist()
        if not infos or len(infos) > _MAX_ARCHIVE_ENTRIES:
            raise ValueError(
                f"invalid MinerU archive entry count: {len(infos)}"
            )
        declared_total = 0
        actual_total = 0
        for info in infos:
            mode = info.external_attr >> 16
            if stat.S_ISLNK(mode):
                raise ValueError(
                    f"symlink is forbidden in MinerU archive: {info.filename!r}"
                )
            declared_total += info.file_size
            if declared_total > _MAX_UNCOMPRESSED_BYTES:
                raise ValueError("MinerU archive exceeds uncompressed size limit")
            destination = _safe_member_path(target, info.filename)
            if info.is_dir():
                destination.mkdir(parents=True, exist_ok=True)
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            with bundle.open(info, "r") as source, destination.open("xb") as output:
                written = 0
                while True:
                    chunk = source.read(1024 * 1024)
                    if not chunk:
                        break
                    written += len(chunk)
                    actual_total += len(chunk)
                    if actual_total > _MAX_UNCOMPRESSED_BYTES:
                        raise ValueError(
                            "MinerU archive exceeds uncompressed size limit"
                        )
                    output.write(chunk)
                if written != info.file_size:
                    raise ValueError(
                        f"MinerU archive member size mismatch: {info.filename!r}"
                    )


def _existing_result(manuscript: Path, extracted_dir: Path) -> ExtractionResult | None:
    if not extracted_dir.is_dir():
        return None
    manifest_path = extracted_dir / "extraction-manifest.json"
    if not manifest_path.is_file():
        return None
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None
    manuscript_digest = _sha256_file(manuscript)
    if manifest.get("manuscript_sha256") != manuscript_digest:
        return None
    full_md = find_full_markdown(extracted_dir)
    full_digest = _sha256_file(full_md)
    if manifest.get("full_markdown_sha256") != full_digest:
        return None
    batch_id = manifest.get("batch_id")
    return ExtractionResult(
        manuscript_pdf=manuscript,
        extracted_dir=extracted_dir,
        full_markdown=full_md,
        manuscript_sha256=manuscript_digest,
        full_markdown_sha256=full_digest,
        batch_id=batch_id if isinstance(batch_id, str) else None,
        reused=True,
    )


def reuse_extraction(manuscript: Path, workfolder: Path) -> ExtractionResult:
    """Require a digest-matching existing extraction without network access."""

    manuscript = manuscript.resolve(strict=True)
    workfolder = workfolder.expanduser().resolve(strict=True)
    result = _existing_result(manuscript, workfolder / "Extractedmd")
    if result is None:
        raise FileNotFoundError(
            "no reusable Extractedmd with a matching extraction-manifest.json"
        )
    return result


def _install_archive(
    archive: Path,
    *,
    manuscript: Path,
    extracted_dir: Path,
    job: MinerUJob,
    model_version: str,
    language: str,
    is_ocr: bool,
    force: bool,
) -> ExtractionResult:
    workfolder = extracted_dir.parent.resolve(strict=True)
    if extracted_dir.name != "Extractedmd" or extracted_dir.parent.resolve() != workfolder:
        raise ValueError("refusing to install outside workfolder/Extractedmd")
    temporary_root = Path(
        tempfile.mkdtemp(prefix=".mineru-extract-", dir=str(workfolder))
    )
    try:
        unpacked = temporary_root / "unpacked"
        _extract_archive(archive, unpacked)
        full_md = find_full_markdown(unpacked)
        content_root = full_md.parent
        manuscript_digest = _sha256_file(manuscript)
        full_digest = _sha256_file(full_md)
        manifest = {
            "schema_version": "1.0",
            "provider": "MinerU",
            "api": "precision-extract-v4",
            "batch_id": job.batch_id,
            "data_id": job.data_id,
            "file_name": job.file_name,
            "submit_trace_id": job.submit_trace_id,
            "result_trace_id": job.result_trace_id,
            "model_version": model_version,
            "language": language,
            "is_ocr": is_ocr,
            "created_at": datetime.now(UTC).isoformat(),
            "manuscript_sha256": manuscript_digest,
            "archive_sha256": _sha256_file(archive),
            "full_markdown": "full.md",
            "full_markdown_sha256": full_digest,
        }
        _atomic_json(content_root / "extraction-manifest.json", manifest)
        previous: Path | None = None
        if extracted_dir.exists():
            if not force:
                raise FileExistsError(
                    f"{extracted_dir} already exists and cannot be safely reused; "
                    "pass --force-ocr to replace only this directory"
                )
            if extracted_dir.is_symlink() or not extracted_dir.is_dir():
                raise ValueError(
                    f"refusing to replace non-directory extraction slot: {extracted_dir}"
                )
            previous = temporary_root / "previous-extraction"
            extracted_dir.replace(previous)
        try:
            content_root.replace(extracted_dir)
        except Exception:
            if previous is not None and previous.exists() and not extracted_dir.exists():
                previous.replace(extracted_dir)
            raise
        return ExtractionResult(
            manuscript_pdf=manuscript,
            extracted_dir=extracted_dir,
            full_markdown=extracted_dir / "full.md",
            manuscript_sha256=manuscript_digest,
            full_markdown_sha256=full_digest,
            batch_id=job.batch_id,
            reused=False,
        )
    finally:
        if temporary_root.exists():
            shutil.rmtree(temporary_root)


def extract_pdf(
    manuscript: Path,
    workfolder: Path,
    *,
    env_file: Path,
    model_version: str = "vlm",
    language: str = "en",
    is_ocr: bool = True,
    timeout_seconds: float = 1800.0,
    poll_interval_seconds: float = 5.0,
    force: bool = False,
    client: MinerUClient | None = None,
) -> ExtractionResult:
    """Run or safely reuse MinerU extraction for one prepared manuscript."""

    manuscript = manuscript.resolve(strict=True)
    workfolder = workfolder.expanduser().resolve(strict=True)
    extracted_dir = workfolder / "Extractedmd"
    reusable = _existing_result(manuscript, extracted_dir)
    if reusable is not None and not force:
        return reusable
    if extracted_dir.exists() and not force:
        raise FileExistsError(
            f"{extracted_dir} exists without a matching extraction manifest; "
            "pass --force-ocr to replace only this directory"
        )
    if client is None:
        client = MinerUClient(load_mineru_token(env_file))
    manuscript_digest = _sha256_file(manuscript)
    data_id = f"manuscript-{manuscript_digest[:32]}"
    job = client.submit_and_wait(
        manuscript,
        data_id=data_id,
        model_version=model_version,
        language=language,
        is_ocr=is_ocr,
        enable_formula=True,
        enable_table=True,
        timeout_seconds=timeout_seconds,
        poll_interval_seconds=poll_interval_seconds,
    )
    temporary_archive = workfolder / f".mineru-{job.batch_id}.zip"
    try:
        client.download(job.full_zip_url, temporary_archive)
        return _install_archive(
            temporary_archive,
            manuscript=manuscript,
            extracted_dir=extracted_dir,
            job=job,
            model_version=model_version,
            language=language,
            is_ocr=is_ocr,
            force=force,
        )
    finally:
        if temporary_archive.exists():
            temporary_archive.unlink()


__all__ = [
    "ExtractionResult",
    "extract_pdf",
    "find_full_markdown",
    "load_mineru_token",
    "prepare_manuscript",
    "reuse_extraction",
]
