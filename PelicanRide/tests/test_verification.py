"""Focused tests for the Pelican Ride domain verifiers."""

# ruff: noqa: D103

from __future__ import annotations

import json
import struct
import sys
import zlib
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest

PELICAN_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = PELICAN_ROOT.parent
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(PELICAN_ROOT))

from clef_case.verification import (  # noqa: E402
    _build_wpf_copy,
    _external_urls,
    _pe_machine,
    _png_dimensions,
    _strict_json,
    build_pelican_registry,
    design_bundle,
    game_spec_bundle,
    review_bundle,
)

from clef_sdk.model import (  # noqa: E402
    ArtifactKind,
    ArtifactRef,
    CheckStatus,
    SessionTask,
    VerifierSpec,
)
from clef_sdk.verification import VerificationContext  # noqa: E402


def _artifact(path: Path) -> ArtifactRef:
    return ArtifactRef(
        uri=str(path.resolve()),
        description=path.name,
        kind=ArtifactKind.DIRECTORY,
    )


def _context(tmp_path: Path, output_name: str, bundle: Path) -> VerificationContext:
    task = SessionTask(
        id="verify-pelican",
        domain_function="pelican.verify.v1",
    )
    return VerificationContext(
        task=task,
        workspace=tmp_path.resolve(),
        outputs={output_name: _artifact(bundle)},
    )


def _png_chunk(kind: bytes, data: bytes) -> bytes:
    payload = kind + data
    return (
        struct.pack(">I", len(data))
        + payload
        + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
    )


def _write_png(path: Path, width: int, height: int) -> None:
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    pixel = b"\x2a\x73\xb8\xff"
    scanlines = (b"\x00" + pixel * width) * height
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", header)
        + _png_chunk(b"IDAT", zlib.compress(scanlines))
        + _png_chunk(b"IEND", b"")
    )


def _write_design_bundle(root: Path) -> None:
    root.mkdir()
    root.joinpath("creative-brief.md").write_text(
        (
            "# Pelican Bicycle Visual Direction\n\n"
            "A cheerful pelican balances naturally on a bicycle while its wings "
            "and beak preserve a readable silhouette. The bike frame, two wheels, "
            "pedals, legs, and seated body form one coherent side-view pose. "
        )
        * 3,
        encoding="utf-8",
    )
    root.joinpath("pelican-reference.svg").write_text(
        """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 240"
        role="img" aria-labelledby="pelican-bicycle-title">
        <title id="pelican-bicycle-title">Pelican riding a bicycle</title>
        <desc>A semantic pelican and bicycle reference drawing</desc>
        <g id="bicycle"><circle cx="100" cy="180" r="45"/>
          <circle cx="280" cy="180" r="45"/><path d="M100 180L180 100L280 180Z"/></g>
        <g id="pelican"><ellipse cx="190" cy="75" rx="42" ry="55"/></g>
        </svg>""",
        encoding="utf-8",
    )
    root.joinpath("palette.json").write_text(
        json.dumps(
            {
                "background": ["#7DD3FC", "#16324F"],
                "pelican": ["#FFF8E7", "#FF6B35"],
                "bicycle": {"frame": "#16324F", "rim": "#FFF8E7"},
                "contrast_usage": "Dark ink on pale panels.",
            }
        ),
        encoding="utf-8",
    )


def test_strict_json_rejects_duplicate_keys_and_non_finite(tmp_path: Path) -> None:
    duplicate = tmp_path / "duplicate.json"
    duplicate.write_text('{"accent":"#fff","accent":"#000"}', encoding="utf-8")
    with pytest.raises(ValueError, match="duplicate JSON key"):
        _strict_json(duplicate)

    non_finite = tmp_path / "nan.json"
    non_finite.write_text('{"value":NaN}', encoding="utf-8")
    with pytest.raises(ValueError, match="non-finite"):
        _strict_json(non_finite)


def test_png_dimensions_reads_ihdr_without_pillow(tmp_path: Path) -> None:
    image = tmp_path / "preview.png"
    _write_png(image, 1440, 900)
    assert _png_dimensions(image) == (1440, 900)

    image.write_bytes(b"not a png")
    with pytest.raises(ValueError, match="invalid PNG"):
        _png_dimensions(image)


def test_pe_header_validation(tmp_path: Path) -> None:
    executable = tmp_path / "tiny.exe"
    data = bytearray(512)
    data[:2] = b"MZ"
    data[0x3C:0x40] = struct.pack("<I", 0x80)
    data[0x80:0x84] = b"PE\0\0"
    data[0x84:0x86] = struct.pack("<H", 0x8664)
    executable.write_bytes(data)
    assert _pe_machine(executable) == 0x8664


def test_standard_wpf_namespaces_are_not_external_dependencies() -> None:
    xaml = (
        '<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" '
        'xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"/>'
    )
    assert _external_urls(xaml) == []
    assert _external_urls('source="https://example.invalid/image.png"') == [
        "https://example.invalid/image.png"
    ]


def test_design_bundle_passes_semantic_artifacts(tmp_path: Path) -> None:
    bundle = tmp_path / "design_bundle"
    _write_design_bundle(bundle)
    context = _context(tmp_path, "design_bundle", bundle)

    result = design_bundle(VerifierSpec("design_bundle"), context)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_design_bundle_rejects_non_semantic_svg_and_duplicate_palette(
    tmp_path: Path,
) -> None:
    bundle = tmp_path / "design_bundle"
    _write_design_bundle(bundle)
    bundle.joinpath("pelican-reference.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg"><script>bad()</script></svg>',
        encoding="utf-8",
    )
    bundle.joinpath("palette.json").write_text(
        '{"ink":"#000","ink":"#fff"}',
        encoding="utf-8",
    )
    context = _context(tmp_path, "design_bundle", bundle)

    result = design_bundle(VerifierSpec("design_bundle"), context)

    assert result.status is CheckStatus.FAILED
    problems = list(result.details["problems"])
    assert any("semantic" in value or "labeling" in value for value in problems)
    assert any("duplicate JSON key" in value for value in problems)


def _write_game_spec(root: Path, acceptance: dict[str, Any]) -> None:
    root.mkdir()
    root.joinpath("architecture.md").write_text(
        (
            "# PelicanRide WPF Architecture\n\n"
            "The dependency-free game targets net8.0-windows with WPF. A Canvas "
            "hosts a deterministic scene and animation loop. Release uses a "
            "self-contained single-file executable. Automation exposes "
            "`--smoke-test` and `--render-preview PATH` without opening a window. "
            "Input, simulation, rendering, score state, and accessibility are "
            "separate components so acceptance criteria stay directly testable. "
        )
        * 2,
        encoding="utf-8",
    )
    root.joinpath("acceptance.json").write_text(
        json.dumps(acceptance, indent=2),
        encoding="utf-8",
    )


def test_game_spec_requires_controls_and_requirements(tmp_path: Path) -> None:
    bundle = tmp_path / "game_spec_bundle"
    _write_game_spec(
        bundle,
        {
            "title": "Pelican Bicycle Ride",
            "controls": {
                "accelerate": ["D", "Right"],
                "brake": ["A", "Left"],
                "jump_or_flap": ["Space", "W", "Up"],
                "boost": ["LeftShift"],
                "pause": ["Escape", "P"],
                "restart": ["R"],
                "mute": ["M"],
                "reduced_motion": ["V"],
                "fullscreen": ["F11"],
            },
            "features": [
                "The pelican remains visibly seated on the bicycle.",
                "Score and speed are readable.",
            ],
            "technical_requirements": ["Offline WPF single-file build."],
            "acceptance_tests": [
                {
                    "id": "VIS-001",
                    "requirement": "Pelican bicycle pose is readable.",
                }
            ],
        },
    )
    context = _context(tmp_path, "game_spec_bundle", bundle)

    result = game_spec_bundle(VerifierSpec("game_spec_bundle"), context)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_game_spec_rejects_empty_machine_contract(tmp_path: Path) -> None:
    bundle = tmp_path / "game_spec_bundle"
    _write_game_spec(bundle, {"controls": {}, "requirements": []})
    context = _context(tmp_path, "game_spec_bundle", bundle)

    result = game_spec_bundle(VerifierSpec("game_spec_bundle"), context)

    assert result.status is CheckStatus.FAILED
    problems = list(result.details["problems"])
    assert any("non-empty controls" in value for value in problems)
    assert any("non-empty requirements" in value for value in problems)


def test_review_accepts_pass_with_polish_and_acceptance_results(
    tmp_path: Path,
) -> None:
    bundle = tmp_path / "review_bundle"
    bundle.mkdir()
    bundle.joinpath("review.json").write_text(
        json.dumps(
            {
                "verdict": "PASS",
                "blocking_issues": [],
                "polish_issues": [
                    {
                        "id": "POLISH-1",
                        "severity": "minor",
                        "evidence": "Cloud edge can be softer.",
                    }
                ],
                "acceptance_results": [
                    {
                        "id": "VIS-001",
                        "status": "pass",
                        "evidence": "Two wheels and rider contact are visible.",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    bundle.joinpath("review.md").write_text(
        (
            "# Review\n\n"
            "Composition, pelican anatomy, bicycle topology, rider contact, "
            "animation coupling, UI and reliability were inspected. "
        )
        * 3,
        encoding="utf-8",
    )
    _write_png(bundle / "preview.png", 1440, 900)
    context = _context(tmp_path, "review_bundle", bundle)

    result = review_bundle(VerifierSpec("review_bundle"), context)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_review_accepts_honest_blocked_artifact(tmp_path: Path) -> None:
    bundle = tmp_path / "review_bundle"
    bundle.mkdir()
    bundle.joinpath("review.json").write_text(
        json.dumps(
            {
                "verdict": "BLOCKED",
                "blocking_issues": [
                    {
                        "id": "BLOCK-1",
                        "severity": "blocker",
                        "evidence": "Smoke test exits non-zero.",
                    }
                ],
                "polish_issues": [],
                "acceptance_results": [
                    {
                        "id": "RUN-001",
                        "status": "failed",
                        "evidence": "Exit code 1.",
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    bundle.joinpath("review.md").write_text(
        (
            "# Blocked Review\n\n"
            "The build evidence was inspected honestly. The delivery node must "
            "repair the smoke-test blocker before packaging the final product. "
        )
        * 3,
        encoding="utf-8",
    )
    _write_png(bundle / "preview.png", 1440, 900)
    context = _context(tmp_path, "review_bundle", bundle)

    result = review_bundle(VerifierSpec("review_bundle"), context)

    assert result.status is CheckStatus.PASSED, result.details["problems"]


def test_temporary_wpf_build_also_runs_smoke_test(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = tmp_path / "Source"
    source.mkdir()
    source.joinpath("PelicanRide.csproj").write_text(
        (
            "<Project Sdk=\"Microsoft.NET.Sdk\">"
            "<PropertyGroup><TargetFramework>net8.0-windows</TargetFramework>"
            "</PropertyGroup></Project>"
        ),
        encoding="utf-8",
    )
    commands: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: Any) -> SimpleNamespace:
        commands.append(command)
        return SimpleNamespace(returncode=0, stdout="ok", stderr="")

    monkeypatch.setattr("clef_case.verification.shutil.which", lambda _name: "dotnet")
    monkeypatch.setattr("clef_case.verification.subprocess.run", fake_run)

    problems, details = _build_wpf_copy(source)

    assert problems == []
    assert details["build_returncode"] == 0
    assert details["smoke_returncode"] == 0
    assert commands[0][1] == "build"
    assert commands[1][1] == "run"
    assert commands[1][-1] == "--smoke-test"


def test_registry_contains_builtins_and_all_domain_verifiers() -> None:
    names = set(build_pelican_registry().names())
    assert {
        "file_exists",
        "design_bundle",
        "game_spec_bundle",
        "wpf_source_bundle",
        "review_bundle",
        "delivery_bundle",
    }.issubset(names)
