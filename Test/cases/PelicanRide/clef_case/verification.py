"""Deterministic verifiers for the Pelican Ride showcase workflow.

The verifiers deliberately inspect concrete artifacts instead of trusting an
agent-authored claim of success.  The two executable checks run only copies in
temporary directories so verification cannot add ``bin``/``obj`` or other
runtime state to an artifact that the framework is about to publish.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET
import zlib
from collections.abc import Mapping
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from clef_sdk.model import (
    ArtifactKind,
    ArtifactRef,
    CheckResult,
    CheckStatus,
    VerifierSpec,
)
from clef_sdk.verification import (
    VerificationContext,
    default_registry,
    digest_path,
    uri_to_path,
)

_PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
_HEX_COLOR = re.compile(r"^#[0-9a-fA-F]{6}$")
_SHA256 = re.compile(r"^(?:sha256:)?([0-9a-fA-F]{64})$")
_URL = re.compile(r"https?://[^\s\"'<>]+", re.IGNORECASE)
_ALLOWED_XAML_NAMESPACES = {
    "http://schemas.microsoft.com/expression/blend/2008",
    "http://schemas.microsoft.com/winfx/2006/xaml",
    "http://schemas.microsoft.com/winfx/2006/xaml/presentation",
    "http://schemas.openxmlformats.org/markup-compatibility/2006",
    "http://www.w3.org/2000/svg",
}
_PASS_STATUSES = {"pass", "passed", "approved", "success", "succeeded", "ok"}
_MAX_PNG_BYTES = 128 * 1024 * 1024
_MAX_PNG_RAW_BYTES = 512 * 1024 * 1024


def _strict_json(path: Path) -> Mapping[str, Any]:
    """Read a strict JSON object, rejecting duplicates and non-finite numbers."""

    def reject_constant(value: str) -> None:
        raise ValueError(f"non-finite JSON number: {value}")

    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    with path.open("r", encoding="utf-8") as stream:
        value = json.load(
            stream,
            parse_constant=reject_constant,
            object_pairs_hook=unique_object,
        )
    if not isinstance(value, dict):
        raise ValueError("JSON root must be an object")
    return value


def _png_dimensions(path: Path) -> tuple[int, int]:
    """Fully validate a non-interlaced color PNG and return its dimensions."""
    size = path.stat().st_size
    if size < 100:
        raise ValueError("invalid PNG: file is too small to contain image data")
    if size > _MAX_PNG_BYTES:
        raise ValueError(f"invalid PNG: file exceeds {_MAX_PNG_BYTES} bytes")

    chunks: list[tuple[bytes, bytes]] = []
    with path.open("rb") as stream:
        if stream.read(8) != _PNG_SIGNATURE:
            raise ValueError("invalid PNG signature")
        while True:
            length_bytes = stream.read(4)
            if not length_bytes:
                raise ValueError("invalid PNG: missing IEND chunk")
            if len(length_bytes) != 4:
                raise ValueError("invalid PNG: truncated chunk length")
            length = struct.unpack(">I", length_bytes)[0]
            if length > _MAX_PNG_BYTES:
                raise ValueError("invalid PNG: unreasonable chunk length")
            kind = stream.read(4)
            data = stream.read(length)
            crc_bytes = stream.read(4)
            if len(kind) != 4 or len(data) != length or len(crc_bytes) != 4:
                raise ValueError("invalid PNG: truncated chunk")
            if not all(
                65 <= value <= 90 or 97 <= value <= 122 for value in kind
            ):
                raise ValueError("invalid PNG: malformed chunk type")
            expected_crc = zlib.crc32(kind + data) & 0xFFFFFFFF
            actual_crc = struct.unpack(">I", crc_bytes)[0]
            if actual_crc != expected_crc:
                name = kind.decode("ascii", errors="replace")
                raise ValueError(f"invalid PNG: CRC mismatch in {name}")
            chunks.append((kind, data))
            if kind == b"IEND":
                if stream.read(1):
                    raise ValueError("invalid PNG: trailing bytes after IEND")
                break

    if not chunks or chunks[0][0] != b"IHDR" or len(chunks[0][1]) != 13:
        raise ValueError("invalid PNG: first chunk must be a 13-byte IHDR")
    if sum(kind == b"IHDR" for kind, _data in chunks) != 1:
        raise ValueError("invalid PNG: IHDR must occur exactly once")
    if sum(kind == b"IEND" for kind, _data in chunks) != 1:
        raise ValueError("invalid PNG: IEND must occur exactly once")
    if chunks[-1] != (b"IEND", b""):
        raise ValueError("invalid PNG: IEND must be the final empty chunk")

    header = chunks[0][1]
    width, height, bit_depth, color_type, compression, filter_method, interlace = (
        struct.unpack(">IIBBBBB", header)
    )
    if width <= 0 or height <= 0:
        raise ValueError("PNG width and height must be positive")
    if width > 32768 or height > 32768:
        raise ValueError("invalid PNG: unreasonable image dimensions")
    allowed_depths = {
        2: {8, 16},
        3: {1, 2, 4, 8},
        6: {8, 16},
    }
    if color_type not in allowed_depths:
        raise ValueError("PNG preview must contain RGB, indexed-color, or RGBA data")
    if bit_depth not in allowed_depths[color_type]:
        raise ValueError(
            f"unsupported PNG bit depth {bit_depth} for color type {color_type}"
        )
    if compression != 0 or filter_method != 0 or interlace != 0:
        raise ValueError("invalid PNG IHDR method fields")

    critical = {b"IHDR", b"PLTE", b"IDAT", b"IEND"}
    for kind, _data in chunks:
        if kind[:1].isupper() and kind not in critical:
            raise ValueError(
                "invalid PNG: unsupported critical chunk "
                f"{kind.decode('ascii', errors='replace')}"
            )
    idat_indexes = [
        index for index, (kind, _data) in enumerate(chunks) if kind == b"IDAT"
    ]
    if not idat_indexes:
        raise ValueError("invalid PNG: missing IDAT image data")
    if idat_indexes != list(range(idat_indexes[0], idat_indexes[-1] + 1)):
        raise ValueError("invalid PNG: IDAT chunks must be consecutive")
    compressed = b"".join(chunks[index][1] for index in idat_indexes)
    if len(compressed) < 32:
        raise ValueError("invalid PNG: compressed image data is implausibly short")
    if color_type == 3:
        palettes = [
            data for kind, data in chunks[: idat_indexes[0]] if kind == b"PLTE"
        ]
        if len(palettes) != 1 or not 3 <= len(palettes[0]) <= 768:
            raise ValueError("invalid PNG: indexed color requires a valid PLTE")
        if len(palettes[0]) % 3:
            raise ValueError("invalid PNG: PLTE length must be divisible by three")

    channels = {2: 3, 3: 1, 6: 4}[color_type]
    row_bytes = (width * channels * bit_depth + 7) // 8
    expected_raw = (row_bytes + 1) * height
    if expected_raw > _MAX_PNG_RAW_BYTES:
        raise ValueError("invalid PNG: decompressed image would be too large")
    try:
        inflater = zlib.decompressobj()
        raw = inflater.decompress(compressed, expected_raw + 1)
        if len(raw) > expected_raw or inflater.unconsumed_tail:
            raise ValueError("invalid PNG: decompressed data exceeds expected size")
        raw += inflater.flush()
    except zlib.error as exc:
        raise ValueError(f"invalid PNG: IDAT zlib stream failed: {exc}") from exc
    if not inflater.eof or inflater.unused_data:
        raise ValueError("invalid PNG: incomplete or concatenated zlib stream")
    if len(raw) != expected_raw:
        raise ValueError(
            "invalid PNG: decompressed byte count does not match IHDR dimensions"
        )
    for row in range(height):
        if raw[row * (row_bytes + 1)] not in range(5):
            raise ValueError(f"invalid PNG: bad filter byte on row {row}")
    return width, height


def _pe_machine(path: Path) -> int:
    """Validate DOS/PE signatures and return the COFF machine identifier."""
    size = path.stat().st_size
    if size < 70:
        raise ValueError("file is too small to contain DOS and PE headers")
    with path.open("rb") as stream:
        dos = stream.read(64)
        if dos[:2] != b"MZ":
            raise ValueError("missing MZ header")
        pe_offset = struct.unpack("<I", dos[0x3C:0x40])[0]
        if pe_offset < 64 or pe_offset + 6 > size:
            raise ValueError("invalid PE header offset")
        stream.seek(pe_offset)
        pe_header = stream.read(6)
    if pe_header[:4] != b"PE\0\0":
        raise ValueError("missing PE signature")
    return struct.unpack("<H", pe_header[4:6])[0]


def _output_directory(
    spec: VerifierSpec,
    context: VerificationContext,
    default_name: str,
) -> tuple[ArtifactRef, Path, list[str]]:
    output_name = spec.parameters.get("output", default_name)
    if not isinstance(output_name, str) or not output_name:
        raise ValueError("output parameter must be a non-empty string")
    artifact = context.outputs.get(output_name)
    if artifact is None:
        raise ValueError(f"missing declared output: {output_name}")

    path = uri_to_path(artifact.uri).resolve(strict=False)
    workspace = context.workspace.resolve(strict=False)
    if not path.is_relative_to(workspace):
        raise ValueError(f"output escapes task workspace: {path}")

    problems: list[str] = []
    if artifact.kind is not ArtifactKind.DIRECTORY:
        problems.append(
            f"{output_name} must be ArtifactKind.DIRECTORY, got {artifact.kind.value}"
        )
    if not path.exists():
        problems.append(f"output directory does not exist: {path}")
    elif not path.is_dir():
        problems.append(f"output is not a directory: {path}")
    return artifact, path, problems


def _required_entry(
    root: Path,
    relative_path: str,
    *,
    directory: bool = False,
) -> tuple[Path, str | None]:
    path = root / relative_path
    if not path.exists():
        return path, f"missing required entry: {relative_path}"
    if path.is_symlink():
        return path, f"required entry must not be a symbolic link: {relative_path}"
    if directory and not path.is_dir():
        return path, f"required entry must be a directory: {relative_path}"
    if not directory and not path.is_file():
        return path, f"required entry must be a file: {relative_path}"
    return path, None


def _result(
    name: str,
    problems: list[str],
    artifact: ArtifactRef,
    required: bool,
    details: Mapping[str, Any] | None = None,
) -> CheckResult:
    result_details: dict[str, Any] = {"problems": problems[:100]}
    if details is not None:
        result_details.update(details)
    return CheckResult(
        name=name,
        status=CheckStatus.PASSED if not problems else CheckStatus.FAILED,
        message=(
            f"{name} passed"
            if not problems
            else "; ".join(problems[:8])
        ),
        required=required,
        score=1.0 if not problems else 0.0,
        evidence=(artifact,),
        details=result_details,
    )


def _json_text(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True).casefold()


def _hex_colors(value: Any) -> list[str]:
    if isinstance(value, str):
        return [value] if _HEX_COLOR.fullmatch(value) else []
    if isinstance(value, dict):
        return [
            color
            for item in value.values()
            for color in _hex_colors(item)
        ]
    if isinstance(value, list):
        return [color for item in value for color in _hex_colors(item)]
    return []


def _meaningful_collection(value: Any) -> bool:
    return (
        (isinstance(value, list) and len(value) >= 1)
        or (isinstance(value, dict) and len(value) >= 1)
    )


def _svg_problems(path: Path) -> list[str]:
    problems: list[str] = []
    try:
        source = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        return [f"cannot read pelican-reference.svg: {exc}"]
    if "<!DOCTYPE" in source.upper():
        problems.append("SVG must not contain a DOCTYPE")
    try:
        root = ET.fromstring(source)
    except ET.ParseError as exc:
        return [*problems, f"invalid SVG XML: {exc}"]
    local_root = root.tag.rsplit("}", 1)[-1].casefold()
    if local_root != "svg":
        problems.append("pelican-reference.svg root element must be <svg>")
    if "viewBox" not in root.attrib and not (
        "width" in root.attrib and "height" in root.attrib
    ):
        problems.append("SVG root needs viewBox or explicit width and height")

    tags = {node.tag.rsplit("}", 1)[-1].casefold() for node in root.iter()}
    if not tags.intersection(
        {"path", "circle", "ellipse", "rect", "polygon", "polyline", "line"}
    ):
        problems.append("SVG contains no basic vector drawing element")
    if tags.intersection({"script", "foreignobject"}):
        problems.append("SVG must not contain scripts or foreignObject content")

    semantics = ET.tostring(root, encoding="unicode").casefold()
    if "pelican" not in semantics:
        problems.append("SVG lacks pelican semantic labeling")
    if "bicycle" not in semantics and "bike" not in semantics:
        problems.append("SVG lacks bicycle semantic labeling")
    for node in root.iter():
        for key, value in node.attrib.items():
            if key.rsplit("}", 1)[-1].casefold() == "href" and _URL.search(value):
                problems.append("SVG must not reference an external URL")
    return problems


def design_bundle(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Validate the creative brief, semantic SVG, and strict color palette."""
    artifact, root, problems = _output_directory(
        spec, context, "design_bundle"
    )
    details: dict[str, Any] = {}
    if not root.is_dir():
        return _result("design_bundle", problems, artifact, spec.required)

    brief, error = _required_entry(root, "creative-brief.md")
    if error:
        problems.append(error)
    else:
        text = brief.read_text(encoding="utf-8", errors="replace")
        folded = text.casefold()
        if len(text.strip()) < 200:
            problems.append("creative-brief.md must contain at least 200 characters")
        if "pelican" not in folded:
            problems.append("creative brief does not discuss the pelican")
        if "bicycle" not in folded and "bike" not in folded:
            problems.append("creative brief does not discuss the bicycle")

    svg, error = _required_entry(root, "pelican-reference.svg")
    if error:
        problems.append(error)
    else:
        problems.extend(_svg_problems(svg))

    palette, error = _required_entry(root, "palette.json")
    if error:
        problems.append(error)
    else:
        try:
            document = _strict_json(palette)
            colors = _hex_colors(document)
            if not document:
                problems.append("palette.json must not be empty")
            if not colors:
                problems.append(
                    "palette.json must contain at least one hexadecimal color"
                )
            details["palette_entries"] = len(document)
        except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as exc:
            problems.append(f"palette.json is not strict JSON: {exc}")
    return _result(
        "design_bundle", problems, artifact, spec.required, details
    )


def game_spec_bundle(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Validate architecture and machine-readable gameplay acceptance criteria."""
    artifact, root, problems = _output_directory(
        spec, context, "game_spec_bundle"
    )
    details: dict[str, Any] = {}
    if not root.is_dir():
        return _result("game_spec_bundle", problems, artifact, spec.required)

    architecture, error = _required_entry(root, "architecture.md")
    architecture_text = ""
    if error:
        problems.append(error)
    else:
        architecture_text = architecture.read_text(
            encoding="utf-8", errors="replace"
        )
        folded = architecture_text.casefold()
        if len(architecture_text.strip()) < 300:
            problems.append("architecture.md must contain at least 300 characters")
        architectural_requirements = {
            "WPF": "wpf",
            ".NET 8 Windows target": "net8.0-windows",
            "smoke-test CLI": "--smoke-test",
            "preview-render CLI": "--render-preview",
        }
        for description, token in architectural_requirements.items():
            if token not in folded:
                problems.append(f"architecture.md omits {description}")
        if not any(token in folded for token in ("single-file", "single file")):
            problems.append("architecture.md omits single-file delivery")

    acceptance, error = _required_entry(root, "acceptance.json")
    if error:
        problems.append(error)
    else:
        try:
            document = _strict_json(acceptance)
            controls = document.get("controls")
            requirements = document.get("requirements")
            if requirements is None:
                requirements = {
                    key: document[key]
                    for key in (
                        "features",
                        "visual_requirements",
                        "technical_requirements",
                        "diagnostics",
                        "acceptance_tests",
                    )
                    if key in document
                }
            if not _meaningful_collection(controls):
                problems.append(
                    "acceptance.json needs a non-empty controls object or array"
                )
            if not _meaningful_collection(requirements):
                problems.append(
                    "acceptance.json needs a non-empty requirements object or array"
                )
            combined = _json_text(document)
            if "pelican" not in combined:
                problems.append("acceptance.json omits the pelican")
            if "bicycle" not in combined and "bike" not in combined:
                problems.append("acceptance.json omits the bicycle")
            control_words = {
                token
                for token in (
                    "keyboard",
                    "mouse",
                    "accelerate",
                    "brake",
                    "pedal",
                    "steer",
                    "jump",
                    "flap",
                    "boost",
                    "pause",
                    "restart",
                    "mute",
                    "reduced_motion",
                    "fullscreen",
                )
                if token in combined
            }
            if len(control_words) < 3:
                problems.append(
                    "acceptance.json must define at least three meaningful controls"
                )
            details["control_concepts"] = sorted(control_words)
            details["requirement_count"] = (
                len(requirements)
                if isinstance(requirements, (dict, list))
                else 0
            )
        except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as exc:
            problems.append(f"acceptance.json is not strict JSON: {exc}")
    return _result(
        "game_spec_bundle", problems, artifact, spec.required, details
    )


def _xml_values(root: ET.Element, local_name: str) -> list[str]:
    return [
        (node.text or "").strip()
        for node in root.iter()
        if node.tag.rsplit("}", 1)[-1] == local_name
    ]


def _external_urls(text: str) -> list[str]:
    return sorted(
        {
            match.group(0).rstrip("/>")
            for match in _URL.finditer(text)
            if match.group(0).rstrip("/>") not in _ALLOWED_XAML_NAMESPACES
        }
    )


def _inspect_wpf_source(root: Path) -> tuple[list[str], dict[str, Any]]:
    problems: list[str] = []
    details: dict[str, Any] = {}
    projects = sorted(root.rglob("*.csproj"))
    if len(projects) != 1:
        problems.append(
            f"source bundle must contain exactly one .csproj, found {len(projects)}"
        )
        return problems, details

    project = projects[0]
    try:
        project_xml = ET.parse(project).getroot()
    except (OSError, ET.ParseError) as exc:
        problems.append(f"invalid project XML: {exc}")
        return problems, details

    targets = _xml_values(project_xml, "TargetFramework")
    if targets != ["net8.0-windows"]:
        problems.append(
            "project TargetFramework must be exactly net8.0-windows"
        )
    use_wpf = [value.casefold() for value in _xml_values(project_xml, "UseWPF")]
    if "true" not in use_wpf:
        problems.append("project must set <UseWPF>true</UseWPF>")
    package_references = [
        node
        for node in project_xml.iter()
        if node.tag.rsplit("}", 1)[-1] == "PackageReference"
    ]
    if package_references:
        problems.append("project must not contain PackageReference elements")

    cs_files = sorted(root.rglob("*.cs"))
    xaml_files = sorted(root.rglob("*.xaml"))
    details["project"] = str(project.relative_to(root))
    details["cs_files"] = len(cs_files)
    details["xaml_files"] = len(xaml_files)
    if len(cs_files) < 3:
        problems.append("source bundle must contain at least three C# files")
    if len(xaml_files) < 2:
        problems.append("source bundle must contain at least two XAML files")

    source_files = [
        *cs_files,
        *xaml_files,
        project,
        *root.rglob("*.props"),
        *root.rglob("*.targets"),
        *root.rglob("*.json"),
    ]
    combined_parts: list[str] = []
    external_urls: set[str] = set()
    for path in dict.fromkeys(source_files):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError as exc:
            problems.append(f"cannot read {path.relative_to(root)}: {exc}")
            continue
        combined_parts.append(text)
        external_urls.update(_external_urls(text))
    if external_urls:
        problems.append(
            f"source contains external URLs: {sorted(external_urls)[:3]}"
        )

    combined = "\n".join(combined_parts).casefold()
    keyword_groups = {
        "PelicanRide identity": ("pelicanride", "pelican ride"),
        "pelican renderer/model": ("pelican",),
        "bicycle renderer/model": ("bicycle", "bike"),
        "WPF scene surface": ("canvas", "drawingcontext"),
        "animation loop": ("compositiontarget", "dispatchertimer"),
        "smoke-test CLI": ("--smoke-test",),
        "preview-render CLI": ("--render-preview",),
    }
    for description, choices in keyword_groups.items():
        if not any(choice in combined for choice in choices):
            problems.append(f"source omits {description}")
    return problems, details


def _build_wpf_copy(root: Path) -> tuple[list[str], dict[str, Any]]:
    dotnet = shutil.which("dotnet")
    if dotnet is None:
        return ["dotnet executable is not available"], {}

    with tempfile.TemporaryDirectory(prefix="pelican-wpf-verify-") as temp:
        copied = Path(temp) / "Source"
        shutil.copytree(
            root,
            copied,
            ignore=shutil.ignore_patterns("bin", "obj", ".git", ".vs"),
        )
        projects = sorted(copied.rglob("*.csproj"))
        if len(projects) != 1:
            return [
                "temporary source copy does not contain exactly one .csproj"
            ], {}
        environment = dict(os.environ)
        environment["DOTNET_CLI_TELEMETRY_OPTOUT"] = "1"
        environment["DOTNET_SKIP_FIRST_TIME_EXPERIENCE"] = "1"
        try:
            completed = subprocess.run(
                [
                    dotnet,
                    "build",
                    str(projects[0]),
                    "--configuration",
                    "Release",
                    "--nologo",
                    "--verbosity",
                    "minimal",
                    "--property:RestoreIgnoreFailedSources=true",
                ],
                cwd=copied,
                env=environment,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=240,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return ["dotnet build exceeded the 240 second timeout"], {}
        output = f"{completed.stdout}\n{completed.stderr}".strip()
        details = {
            "build_returncode": completed.returncode,
            "build_output_tail": output[-6000:],
        }
        if completed.returncode != 0:
            return [f"dotnet build failed with exit code {completed.returncode}"], details

        try:
            smoke = subprocess.run(
                [
                    dotnet,
                    "run",
                    "--project",
                    str(projects[0]),
                    "--configuration",
                    "Release",
                    "--no-build",
                    "--",
                    "--smoke-test",
                ],
                cwd=copied,
                env=environment,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=30,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return ["dotnet run --smoke-test exceeded the 30 second timeout"], details
        smoke_output = f"{smoke.stdout}\n{smoke.stderr}".strip()
        details["smoke_returncode"] = smoke.returncode
        details["smoke_output_tail"] = smoke_output[-3000:]
        if smoke.returncode != 0:
            return [
                f"dotnet run --smoke-test failed with exit code {smoke.returncode}"
            ], details
        return [], details


def wpf_source_bundle(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Inspect a dependency-free WPF source tree and compile a temporary copy."""
    artifact, root, problems = _output_directory(
        spec, context, "source_bundle"
    )
    details: dict[str, Any] = {}
    if root.is_dir():
        static_problems, static_details = _inspect_wpf_source(root)
        problems.extend(static_problems)
        details.update(static_details)
        if not static_problems:
            build_problems, build_details = _build_wpf_copy(root)
            problems.extend(build_problems)
            details.update(build_details)
    return _result(
        "wpf_source_bundle", problems, artifact, spec.required, details
    )


def _review_has_blocker(document: Mapping[str, Any]) -> bool:
    blocking = document.get("blocking_issues")
    return isinstance(blocking, list) and bool(blocking)


def review_bundle(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Validate review evidence and its generated visual preview."""
    artifact, root, problems = _output_directory(
        spec, context, "review_bundle"
    )
    details: dict[str, Any] = {}
    if not root.is_dir():
        return _result("review_bundle", problems, artifact, spec.required)

    review_json, error = _required_entry(root, "review.json")
    document: Mapping[str, Any] | None = None
    if error:
        problems.append(error)
    else:
        try:
            document = _strict_json(review_json)
            verdict = str(
                document.get("verdict", document.get("status", ""))
            ).casefold()
            allowed_verdicts = {
                "pass",
                "needs_polish",
                "blocked",
            }
            if verdict not in allowed_verdicts:
                problems.append("review.json contains an unknown verdict")
            has_blocker = _review_has_blocker(document)
            if verdict == "pass" and has_blocker:
                problems.append(
                    "review.json passing verdict conflicts with blocking findings"
                )
            if verdict == "blocked" and not has_blocker:
                problems.append(
                    "review.json BLOCKED verdict needs at least one blocking issue"
                )
            if not _meaningful_collection(
                document.get(
                    "checks",
                    document.get(
                        "findings", document.get("acceptance_results")
                    ),
                )
            ):
                problems.append(
                    "review.json needs non-empty checks, findings, "
                    "or acceptance_results"
                )
            details["verdict"] = verdict
        except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as exc:
            problems.append(f"review.json is not strict JSON: {exc}")

    review_md, error = _required_entry(root, "review.md")
    if error:
        problems.append(error)
    else:
        text = review_md.read_text(encoding="utf-8", errors="replace")
        if len(text.strip()) < 200:
            problems.append("review.md must contain at least 200 characters")

    preview, error = _required_entry(root, "preview.png")
    if error:
        problems.append(error)
    else:
        try:
            width, height = _png_dimensions(preview)
            details["preview_dimensions"] = [width, height]
        except (OSError, ValueError, struct.error) as exc:
            problems.append(f"preview.png is invalid: {exc}")
    return _result(
        "review_bundle", problems, artifact, spec.required, details
    )


def _run_executable(
    executable: Path,
    arguments: list[str],
    cwd: Path,
    timeout: int = 30,
) -> tuple[int | None, str, str | None]:
    try:
        completed = subprocess.run(
            [str(executable), *arguments],
            cwd=cwd,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            check=False,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except subprocess.TimeoutExpired:
        return None, "", f"command exceeded the {timeout} second timeout"
    except OSError as exc:
        return None, "", f"could not start executable: {exc}"
    output = f"{completed.stdout}\n{completed.stderr}".strip()
    return completed.returncode, output, None


def _verification_document_problems(document: Mapping[str, Any]) -> list[str]:
    problems: list[str] = []
    passing = document.get("passed")
    status = str(document.get("status", "")).casefold()
    explicit_pass = passing is True or status in {
        "pass",
        "passed",
        "approved",
        "success",
    }
    evidence_fields = (
        "build_exit_code",
        "smoke_exit_code",
        "preview_exit_code",
        "single_file",
        "self_contained",
        "runtime_network_required",
        "loose_runtime_files",
    )
    has_command_evidence = all(key in document for key in evidence_fields)
    if not explicit_pass and not has_command_evidence:
        problems.append(
            "verification.json needs a passing status or complete command evidence"
        )
    if has_command_evidence:
        for key in ("build_exit_code", "smoke_exit_code", "preview_exit_code"):
            if document.get(key) != 0:
                problems.append(f"verification.json {key} must equal 0")
        if document.get("single_file") is not True:
            problems.append("verification.json single_file must be true")
        if document.get("self_contained") is not True:
            problems.append("verification.json self_contained must be true")
        if document.get("runtime_network_required") is not False:
            problems.append(
                "verification.json runtime_network_required must be false"
            )
        loose = document.get("loose_runtime_files")
        if loose not in ([], None):
            problems.append(
                "verification.json loose_runtime_files must be an empty array"
            )
    checks = document.get("checks")
    if isinstance(checks, list):
        for index, check in enumerate(checks):
            if not isinstance(check, dict):
                problems.append(f"verification checks[{index}] is not an object")
                continue
            check_status = str(check.get("status", "")).casefold()
            if check_status in {"fail", "failed", "error", "blocked"}:
                problems.append(
                    f"verification checks[{index}] records {check_status}"
                )
    return problems


def delivery_bundle(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Validate and execute the exact, one-click Windows delivery bundle."""
    artifact, root, problems = _output_directory(
        spec, context, "delivery_bundle"
    )
    details: dict[str, Any] = {}
    if not root.is_dir():
        return _result("delivery_bundle", problems, artifact, spec.required)

    expected = {
        "PelicanRide.exe",
        "README.md",
        "Source",
        "preview.png",
        "verification.json",
    }
    actual = {path.name for path in root.iterdir()}
    missing = sorted(expected - actual)
    unexpected = sorted(actual - expected)
    if missing:
        problems.append(f"delivery is missing top-level entries: {missing}")
    if unexpected:
        problems.append(f"delivery has unexpected top-level entries: {unexpected}")

    executable, error = _required_entry(root, "PelicanRide.exe")
    executable_ready = error is None
    if error:
        problems.append(error)
    else:
        size = executable.stat().st_size
        details["executable_bytes"] = size
        if size <= 20 * 1024 * 1024:
            problems.append("PelicanRide.exe must be larger than 20 MiB")
        try:
            details["pe_machine"] = _pe_machine(executable)
        except (OSError, ValueError, struct.error) as exc:
            problems.append(f"PelicanRide.exe is not a valid PE image: {exc}")
            executable_ready = False

    readme, error = _required_entry(root, "README.md")
    if error:
        problems.append(error)
    else:
        text = readme.read_text(encoding="utf-8", errors="replace")
        folded = text.casefold()
        if len(text.strip()) < 200:
            problems.append("README.md must contain at least 200 characters")
        if not any(
            token in folded for token in ("control", "操作", "按键", "玩法")
        ):
            problems.append("README.md must explain the controls")

    verification, error = _required_entry(root, "verification.json")
    if error:
        problems.append(error)
    else:
        try:
            document = _strict_json(verification)
            problems.extend(_verification_document_problems(document))
        except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as exc:
            problems.append(f"verification.json is not strict JSON: {exc}")

    preview, error = _required_entry(root, "preview.png")
    if error:
        problems.append(error)
    else:
        try:
            width, height = _png_dimensions(preview)
            details["preview_dimensions"] = [width, height]
            if width < 1200 or height < 700:
                problems.append(
                    f"preview.png must be at least 1200x700, got {width}x{height}"
                )
        except (OSError, ValueError, struct.error) as exc:
            problems.append(f"preview.png is invalid: {exc}")

    source, error = _required_entry(root, "Source", directory=True)
    if error:
        problems.append(error)
    elif len(list(source.rglob("*.csproj"))) != 1:
        problems.append("Source must contain exactly one .csproj")

    if executable_ready:
        with tempfile.TemporaryDirectory(prefix="pelican-delivery-verify-") as temp:
            temp_root = Path(temp)
            executable_copy = temp_root / "PelicanRide.exe"
            shutil.copy2(executable, executable_copy)

            returncode, output, run_error = _run_executable(
                executable_copy, ["--smoke-test"], temp_root
            )
            details["smoke_returncode"] = returncode
            details["smoke_output_tail"] = output[-3000:]
            if run_error:
                problems.append(f"--smoke-test {run_error}")
            elif returncode != 0:
                problems.append(
                    f"--smoke-test exited with code {returncode}"
                )

            rendered = temp_root / "rendered-preview.png"
            returncode, output, run_error = _run_executable(
                executable_copy,
                ["--render-preview", str(rendered)],
                temp_root,
            )
            details["render_returncode"] = returncode
            details["render_output_tail"] = output[-3000:]
            if run_error:
                problems.append(f"--render-preview {run_error}")
            elif returncode != 0:
                problems.append(
                    f"--render-preview exited with code {returncode}"
                )
            elif not rendered.is_file():
                problems.append("--render-preview did not create the requested PNG")
            else:
                try:
                    width, height = _png_dimensions(rendered)
                    details["rendered_dimensions"] = [width, height]
                    if width < 1200 or height < 700:
                        problems.append(
                            "--render-preview output must be at least "
                            f"1200x700, got {width}x{height}"
                        )
                except (OSError, ValueError, struct.error) as exc:
                    problems.append(f"--render-preview output is invalid: {exc}")

    return _result(
        "delivery_bundle", problems, artifact, spec.required, details
    )


def build_pelican_registry():
    """Return built-ins plus all Pelican Ride domain verifiers."""
    registry = default_registry()
    registry.register("design_bundle", design_bundle)
    registry.register("game_spec_bundle", game_spec_bundle)
    registry.register("wpf_source_bundle", wpf_source_bundle)
    registry.register("review_bundle", review_bundle)
    registry.register("delivery_bundle", delivery_bundle)
    return registry


__all__ = [
    "build_pelican_registry",
    "delivery_bundle",
    "design_bundle",
    "game_spec_bundle",
    "review_bundle",
    "wpf_source_bundle",
]
