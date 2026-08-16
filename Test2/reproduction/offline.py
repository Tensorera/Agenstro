"""Substantive deterministic FakeAdapter for the reproduction DAG."""

from __future__ import annotations

import json
from pathlib import Path
from threading import Lock
from typing import Any

from clef_sdk.model import ArtifactClaim, ArtifactKind, RunState
from clef_sdk.protocol import (
    AgentReport,
    decode_request,
    encode_report_envelope,
)
from clef_sdk.verification import digest_path

from .content import build_all_offline_content, json_text


def _decode_prompt(prompt: str):
    marker = "Canonical AgentRequest JSON:\n"
    if marker not in prompt:
        raise ValueError("prompt does not contain a canonical AgentRequest")
    payload = prompt.split(marker, 1)[1].split("\n\nThe final non-whitespace", 1)[0]
    return decode_request(payload)


def _media_type(kind: ArtifactKind) -> str:
    return "application/json" if kind is ArtifactKind.JSON else "text/markdown"


def _manifest_entries(
    request,
    output_paths: dict[str, Path],
) -> list[dict[str, str]]:
    roles_by_name = {
        "evidence-ledger.json": "evidence_ledger",
        "theory-inference.json": "theory_inference",
        "methods-inference.json": "methods_inference",
        "validation-report.json": "validation_report",
    }
    stable_slots = {
        "evidence_ledger": "Evidence/evidence-ledger.json",
        "theory_inference": "Inference/Theory/theory-inference.json",
        "methods_inference": "Inference/Methods/methods-inference.json",
        "validation_report": "Validation/validation-report.json",
        "inferred_supplement": "Report/inferred-supplement.md",
        "assessment": "Report/reproduction-assessment.json",
    }
    sources_by_role: dict[str, Path] = {}
    for artifact in request.inputs:
        path = Path(artifact.uri).resolve(strict=True)
        role = roles_by_name.get(path.name)
        if role is not None:
            sources_by_role[role] = path
    sources_by_role["inferred_supplement"] = output_paths["inferred_supplement"]
    sources_by_role["assessment"] = output_paths["assessment"]
    if set(sources_by_role) != set(stable_slots):
        raise ValueError("final request does not contain every manifest source")
    return [
        {
            "role": role,
            "path": stable_slots[role],
            "digest": digest_path(sources_by_role[role]),
            "verification": "clef_verified",
        }
        for role in sorted(stable_slots)
    ]


def make_offline_callback(
    pdf_path: Path,
    extracted_markdown: Path,
    *,
    tamper_numeric: bool = False,
    tamper_theory: bool = False,
    tamper_methods: bool = False,
    tamper_final: bool = False,
    tamper_manifest: bool = False,
):
    """Return a callback that writes real deterministic benchmark artifacts."""

    content = build_all_offline_content(pdf_path, extracted_markdown)
    requests_by_workspace: dict[Path, Any] = {}
    request_lock = Lock()

    def callback(
        prompt: str,
        workspace: Path,
        _session_id: str | None,
    ) -> str:
        resolved_workspace = workspace.resolve()
        if "Canonical AgentRequest JSON:\n" in prompt:
            request = _decode_prompt(prompt)
            with request_lock:
                requests_by_workspace[resolved_workspace] = request
        else:
            with request_lock:
                request = requests_by_workspace.get(resolved_workspace)
            if request is None:
                raise ValueError(
                    "repair prompt arrived before a canonical AgentRequest"
                )
        output_paths = {
            output.name: Path(output.path).resolve(strict=False)
            for output in request.expected_outputs
            if output.path is not None
        }
        if len(output_paths) != len(request.expected_outputs):
            raise ValueError("every offline output must have a path")
        for path in output_paths.values():
            if not path.is_relative_to(workspace.resolve()):
                raise ValueError(f"output escapes task workspace: {path}")

        content_by_task: dict[str, dict[str, str]] = {
            "inventory-supplement-evidence": {
                "evidence_report": content["evidence_report"],
                "evidence_ledger": json_text(content["evidence"]),
            },
            "infer-theory-supplement": {
                "theory_report": content["theory_report"],
                "theory_inference": json_text(content["theory"]),
            },
            "infer-methods-supplement": {
                "methods_report": content["methods_report"],
                "methods_inference": json_text(content["methods"]),
            },
            "validate-paper-numerics": {
                "validation_markdown": content["validation_markdown"],
                "validation_report": json_text(content["validation"]),
            },
        }
        if request.task_id == "synthesize-inferred-supplement":
            output_paths["inferred_supplement"].write_text(
                content["supplement"], encoding="utf-8"
            )
            output_paths["assessment"].write_text(
                json_text(content["assessment"]), encoding="utf-8"
            )
            manifest = {
                "schema_version": "1.0",
                "benchmark_id": "test2-blind-supplement-reproduction",
                "entries": _manifest_entries(
                    request,
                    output_paths,
                ),
            }
            output_paths["artifact_manifest"].write_text(
                json_text(manifest), encoding="utf-8"
            )
            if tamper_final:
                with output_paths["inferred_supplement"].open(
                    "a", encoding="utf-8"
                ) as stream:
                    stream.write(
                        "\nFig. S1 quantitatively reproduced from assumed "
                        "mesh values.\n"
                    )
                supplement_entry = next(
                    entry
                    for entry in manifest["entries"]
                    if entry["role"] == "inferred_supplement"
                )
                supplement_entry["digest"] = digest_path(
                    output_paths["inferred_supplement"]
                )
                output_paths["artifact_manifest"].write_text(
                    json_text(manifest), encoding="utf-8"
                )
            if tamper_manifest:
                manifest["entries"][0]["path"] = "Report/nonexistent-impersonated.json"
                manifest["entries"][1]["path"] = "../outside.json"
                manifest["entries"][2]["path"] = "Report/reproduction-assessment.json"
                output_paths["artifact_manifest"].write_text(
                    json_text(manifest), encoding="utf-8"
                )
        else:
            task_content = content_by_task.get(request.task_id)
            if task_content is None:
                raise ValueError(f"unknown offline task: {request.task_id}")
            for name, value in task_content.items():
                output_paths[name].write_text(value, encoding="utf-8")
            if tamper_numeric and request.task_id == "validate-paper-numerics":
                report_path = output_paths["validation_report"]
                report = json.loads(report_path.read_text(encoding="utf-8"))
                target = next(
                    item
                    for item in report["checks"]
                    if item["check_id"] == "NUM-BMAX-001"
                )
                target["observed"]["sinusoidal"] = 9.99
                report_path.write_text(
                    json.dumps(
                        report,
                        ensure_ascii=False,
                        indent=2,
                        sort_keys=True,
                    )
                    + "\n",
                    encoding="utf-8",
                )
            if tamper_theory and request.task_id == "infer-theory-supplement":
                theory_path = output_paths["theory_inference"]
                theory = json.loads(theory_path.read_text(encoding="utf-8"))
                theory["sections"][0]["reconstruction"] = (
                    "Only kappa_3 was recovered; phi and U_2 were assumed "
                    "without evaluating Eq. (54)."
                )
                theory_path.write_text(json_text(theory), encoding="utf-8")
            if tamper_methods and request.task_id == "infer-methods-supplement":
                methods_path = output_paths["methods_inference"]
                methods = json.loads(methods_path.read_text(encoding="utf-8"))
                methods["confirmed_facts"][0]["value"] = (
                    "PET, thickness 40 um, E=5.0 GPa, nu=0.39"
                )
                methods_path.write_text(json_text(methods), encoding="utf-8")

        claims = []
        expected_by_name = {output.name: output for output in request.expected_outputs}
        for name, path in output_paths.items():
            output = expected_by_name[name]
            claims.append(
                ArtifactClaim(
                    name=name,
                    uri=str(path),
                    description=output.description,
                    kind=output.kind,
                    digest=digest_path(path),
                    media_type=_media_type(output.kind),
                )
            )
        return encode_report_envelope(
            AgentReport(
                run_id=request.run_id,
                task_id=request.task_id,
                attempt=request.attempt,
                text="deterministic blind reproduction task completed",
                state=RunState.SUCCEEDED,
                artifacts=tuple(claims),
            )
        )

    return callback


def dump_callback_description() -> str:
    """Describe why the fake path is semantically useful."""

    return json.dumps(
        {
            "adapter": "FakeAdapter",
            "semantic_content": "real deterministic paper checks",
            "network": False,
            "purpose": (
                "exercise protocol, fresh sessions, DAG scheduling, artifact "
                "publication, JSON Schema and domain verifiers"
            ),
        },
        ensure_ascii=False,
        sort_keys=True,
    )


__all__ = ["dump_callback_description", "make_offline_callback"]
