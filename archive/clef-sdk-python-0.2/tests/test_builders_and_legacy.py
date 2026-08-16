from __future__ import annotations

import importlib.util

import pytest

from clef_sdk import (
    Artifact,
    ArtifactKind,
    ArtifactSpec,
    Capability,
    EffectKind,
    Prompt,
    PromptRole,
    SessionTask,
    Task,
    ValidationError,
    Workflow,
    WorkflowPlan,
    convert_legacy_workflow,
)


def _legacy_payload() -> dict[str, object]:
    return {
        "id": "legacy-plan",
        "tasks": {
            "draft": {
                "id": "draft",
                "domain_function": "documents.draft.v1",
                "prompts": [
                    {
                        "role": "instruction",
                        "content": "Create the draft.",
                        "name": "draft",
                        "priority": 10,
                    }
                ],
                "inputs": {},
                "outputs": {
                    "report": {
                        "name": "report",
                        "description": "Draft report",
                        "kind": "text",
                        "path": "out/report.md",
                        "required": True,
                        "constraints": [],
                    }
                },
                "contract": {
                    "effects": {
                        "allowed": [{"kind": "create", "path_glob": "out/report.md"}]
                    }
                },
                "metadata": {"workspace_subdir": "prototype-runtime-only"},
                "effort": "high",
            }
        },
        "bindings": [],
        "outputs": {"report": {"source_task_id": "draft", "output_name": "report"}},
        "policies": {"max_concurrency": 2, "fail_fast": True, "max_fan_out": 8},
    }


def _object_at(payload: dict[str, object], *path: str | int) -> dict[str, object]:
    value: object = payload
    for part in path:
        if isinstance(part, int):
            assert isinstance(value, list)
            value = value[part]
        else:
            assert isinstance(value, dict)
            value = value[part]
    assert isinstance(value, dict)
    return value


def _assert_conversion_required(payload: dict[str, object]) -> ValidationError:
    with (
        pytest.warns(DeprecationWarning, match="deprecated"),
        pytest.raises(ValidationError) as captured,
    ):
        convert_legacy_workflow(payload)
    assert captured.value.code == "LEGACY_CONVERSION_REQUIRED"
    return captured.value


def test_fluent_builders_produce_one_strict_v2_model() -> None:
    task = (
        Task.agent("draft", "documents.draft.v1", "Create the draft.")
        .add_output(Artifact.text("report", "Draft report", "out/report.md"))
        .allow(EffectKind.CREATE, "out/report.md")
        .require(Capability.STREAMING)
    )
    workflow = Workflow("daily-report").add(task).publish("report", "draft", "report")

    assert ArtifactSpec is Artifact
    assert SessionTask is Task
    assert WorkflowPlan is Workflow
    assert workflow.all_required_capabilities == frozenset({Capability.STREAMING})
    assert workflow.to_dict() == {
        "schema_version": "clef.workflow/v2",
        "id": "daily-report",
        "tasks": [task.to_dict()],
        "outputs": [
            {
                "name": "report",
                "source": {"source_task_id": "draft", "output_name": "report"},
            }
        ],
        "policy": {"max_concurrency": 1, "fail_fast": True, "max_fan_out": 32},
        "required_capabilities": [],
    }


@pytest.mark.parametrize(
    "path",
    ["../report.md", "/report.md", r"C:\report.md", ".tactus/report.md"],
)
def test_artifact_paths_are_strictly_workspace_relative(path: str) -> None:
    with pytest.raises(ValidationError, match=r"workspace-relative|dot|reserved"):
        Artifact.text("report", "Draft report", path)


def test_legacy_workflow_conversion_is_explicit_one_way_and_warns() -> None:
    payload = _legacy_payload()

    with pytest.warns(DeprecationWarning, match="deprecated"):
        workflow = convert_legacy_workflow(payload)

    assert workflow.to_dict() == {
        "schema_version": "clef.workflow/v2",
        "id": "legacy-plan",
        "tasks": [
            {
                "id": "draft",
                "domain_function": "documents.draft.v1",
                "prompts": [
                    {
                        "role": "instruction",
                        "content": "Create the draft.",
                        "name": "draft",
                        "priority": 10,
                    }
                ],
                "inputs": [],
                "outputs": [
                    {
                        "name": "report",
                        "description": "Draft report",
                        "kind": "text",
                        "path": "out/report.md",
                        "required": True,
                    }
                ],
                "effects": [{"kind": "create", "path_glob": "out/report.md"}],
                "required_capabilities": [],
                "preferred_capabilities": [],
                "effort": "high",
            }
        ],
        "outputs": [
            {
                "name": "report",
                "source": {"source_task_id": "draft", "output_name": "report"},
            }
        ],
        "policy": {
            "max_concurrency": 2,
            "fail_fast": True,
            "max_fan_out": 8,
        },
        "required_capabilities": [],
    }
    assert workflow.tasks[0].outputs[0].kind is ArtifactKind.TEXT


@pytest.mark.parametrize(
    ("path", "unknown_key"),
    [
        ((), "profile"),
        (("tasks", "draft"), "timeout_seconds"),
        (("policies",), "retry_backoff_seconds"),
        (("tasks", "draft", "contract"), "postconditions"),
        (("tasks", "draft", "contract", "effects"), "denied"),
        (
            ("tasks", "draft", "contract", "effects", "allowed", 0),
            "requires_confirmation",
        ),
        (("tasks", "draft", "prompts", 0), "metadata"),
        (("tasks", "draft", "outputs", "report"), "media_type"),
        (("outputs", "report"), "alias"),
    ],
    ids=(
        "workflow",
        "task",
        "policy",
        "contract",
        "effect-policy",
        "effect-rule",
        "prompt",
        "artifact",
        "workflow-output",
    ),
)
def test_legacy_unknown_fields_fail_closed(
    path: tuple[str | int, ...], unknown_key: str
) -> None:
    payload = _legacy_payload()
    _object_at(payload, *path)[unknown_key] = "not-representable"

    error = _assert_conversion_required(payload)

    assert unknown_key in error.message


def test_legacy_unknown_input_field_fails_closed() -> None:
    payload = _legacy_payload()
    inputs = _object_at(payload, "tasks", "draft", "inputs")
    inputs["source"] = {
        "uri": "source.md",
        "description": "Source document",
        "kind": "text",
        "mount_mode": "copy",
    }

    error = _assert_conversion_required(payload)

    assert "mount_mode" in error.message


def test_legacy_unknown_binding_field_fails_closed() -> None:
    payload = _legacy_payload()
    bindings = payload["bindings"]
    assert isinstance(bindings, list)
    bindings.append(
        {
            "source_task_id": "draft",
            "output_name": "report",
            "target_task_id": "draft",
            "input_name": "source",
            "when": "success",
        }
    )

    error = _assert_conversion_required(payload)

    assert "when" in error.message


def test_legacy_semantic_metadata_is_not_silently_discarded() -> None:
    payload = _legacy_payload()
    metadata = _object_at(payload, "tasks", "draft", "metadata")
    metadata["order"] = 3

    error = _assert_conversion_required(payload)

    assert "order" in error.message


def test_legacy_constraints_are_not_silently_weakened() -> None:
    payload = _legacy_payload()
    artifact = _object_at(payload, "tasks", "draft", "outputs", "report")
    artifact["constraints"] = [{"kind": "max_bytes", "parameters": {}}]

    error = _assert_conversion_required(payload)

    assert "explicit v2 rewrite" in error.message


def test_builder_collections_have_hard_limits() -> None:
    prompt = Prompt(PromptRole.INSTRUCTION, "bounded")
    with pytest.raises(ValidationError, match="prompts cannot exceed 64"):
        Task(
            id="bounded",
            domain_function="documents.bounded.v1",
            prompts=(prompt,) * 65,
        )


def test_legacy_json_rejects_duplicate_keys() -> None:
    payload = '{"id":"first","id":"second","tasks":{}}'
    with (
        pytest.warns(DeprecationWarning),
        pytest.raises(ValidationError, match="legacy JSON is invalid"),
    ):
        convert_legacy_workflow(payload)


def test_removed_python_control_plane_has_no_executable_packages() -> None:
    for name in (
        "clef_sdk.runtime",
        "clef_sdk.storage",
        "clef_sdk.verification",
        "clef_sdk.adapters",
    ):
        spec = importlib.util.find_spec(name)
        # Empty source-tree directories can remain namespace portions, but no
        # loader or package files survive and wheels do not include them.
        assert spec is None or spec.loader is None
