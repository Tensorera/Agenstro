"""Fail-closed one-way conversion from the prototype workflow JSON format."""

from __future__ import annotations

import json
import warnings
from collections.abc import Collection, Mapping, Sequence
from enum import Enum
from typing import cast

from .errors import ValidationError
from .types import (
    Artifact,
    ArtifactBinding,
    ArtifactKind,
    ArtifactRef,
    Effect,
    EffectKind,
    Effort,
    Prompt,
    PromptRole,
    Task,
    TaskInput,
    Workflow,
    WorkflowOutput,
    WorkflowPolicy,
)

_MAX_LEGACY_JSON_BYTES = 4 * 1_024 * 1_024


def _mapping(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} must be an object"
        )
    untyped = cast(Mapping[object, object], value)
    if len(untyped) > 4_096:
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} exceeds 4096 entries"
        )
    if not all(isinstance(key, str) for key in untyped):
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} keys must be strings"
        )
    return cast(Mapping[str, object], untyped)


def _require_allowed_keys(
    data: Mapping[str, object], field_name: str, allowed: Collection[str]
) -> None:
    unsupported = sorted(key for key in data if key not in allowed)
    if unsupported:
        fields = ", ".join(repr(key) for key in unsupported)
        raise ValidationError(
            "LEGACY_CONVERSION_REQUIRED",
            f"{field_name} fields require an explicit v2 rewrite: {fields}",
        )


def _sequence(value: object, field_name: str) -> Sequence[object]:
    if isinstance(value, str | bytes) or not isinstance(value, Sequence):
        raise ValidationError("LEGACY_FORMAT_INVALID", f"{field_name} must be an array")
    sequence = cast(Sequence[object], value)
    if len(sequence) > 4_096:
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} exceeds 4096 items"
        )
    return sequence


def _string(value: object, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} must be a non-empty string"
        )
    return value


def _integer(value: object, field_name: str, default: int) -> int:
    if value is None:
        return default
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} must be an integer"
        )
    return value


def _boolean(value: object, field_name: str, default: bool) -> bool:
    if value is None:
        return default
    if not isinstance(value, bool):
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} must be a boolean"
        )
    return value


def _optional_string(value: object, field_name: str) -> str | None:
    if value is None:
        return None
    return _string(value, field_name)


def _enum[T: Enum](enum_type: type[T], value: object, field_name: str) -> T:
    raw = _string(value, field_name)
    try:
        return enum_type(raw)
    except ValueError as error:
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name} has an unsupported value"
        ) from error


def _artifact(value: object, field_name: str, *, expected_name: str) -> Artifact:
    data = _mapping(value, field_name)
    _require_allowed_keys(
        data,
        field_name,
        ("name", "description", "kind", "path", "required", "constraints"),
    )
    name = _string(data.get("name"), f"{field_name}.name")
    if name != expected_name:
        raise ValidationError(
            "LEGACY_FORMAT_INVALID",
            f"{field_name}.name must match its output key",
        )
    constraints = _sequence(data.get("constraints", ()), f"{field_name}.constraints")
    if constraints:
        raise ValidationError(
            "LEGACY_CONVERSION_REQUIRED",
            f"{field_name}.constraints require an explicit v2 rewrite",
        )
    return Artifact(
        name=name,
        description=_string(data.get("description"), f"{field_name}.description"),
        kind=_enum(ArtifactKind, data.get("kind"), f"{field_name}.kind"),
        path=_optional_string(data.get("path"), f"{field_name}.path"),
        required=_boolean(data.get("required"), f"{field_name}.required", True),
    )


def _prompt(value: object, field_name: str) -> Prompt:
    data = _mapping(value, field_name)
    _require_allowed_keys(data, field_name, ("role", "content", "name", "priority"))
    return Prompt(
        role=_enum(PromptRole, data.get("role"), f"{field_name}.role"),
        content=_string(data.get("content"), f"{field_name}.content"),
        name=_optional_string(data.get("name"), f"{field_name}.name"),
        priority=_integer(data.get("priority"), f"{field_name}.priority", 0),
    )


def _artifact_ref(value: object, field_name: str) -> ArtifactRef:
    data = _mapping(value, field_name)
    _require_allowed_keys(
        data,
        field_name,
        ("uri", "description", "kind", "digest", "media_type"),
    )
    return ArtifactRef(
        uri=_string(data.get("uri"), f"{field_name}.uri"),
        description=_string(data.get("description"), f"{field_name}.description"),
        kind=_enum(ArtifactKind, data.get("kind"), f"{field_name}.kind"),
        digest=_optional_string(data.get("digest"), f"{field_name}.digest"),
        media_type=_optional_string(data.get("media_type"), f"{field_name}.media_type"),
    )


def _binding(
    value: object, field_name: str, *, explicit_edge: bool = False
) -> ArtifactBinding:
    data = _mapping(value, field_name)
    allowed: tuple[str, ...] = ("source_task_id", "output_name")
    if explicit_edge:
        allowed = (*allowed, "target_task_id", "input_name")
    _require_allowed_keys(data, field_name, allowed)
    return ArtifactBinding(
        source_task_id=_string(
            data.get("source_task_id"), f"{field_name}.source_task_id"
        ),
        output_name=_string(data.get("output_name"), f"{field_name}.output_name"),
    )


def _effects(contract: Mapping[str, object], field_name: str) -> tuple[Effect, ...]:
    _require_allowed_keys(contract, field_name, ("effects",))
    effects = _mapping(contract.get("effects", {}), f"{field_name}.effects")
    _require_allowed_keys(effects, f"{field_name}.effects", ("allowed",))
    allowed = _sequence(effects.get("allowed", ()), f"{field_name}.effects.allowed")
    result: list[Effect] = []
    for index, value in enumerate(allowed):
        item_name = f"{field_name}.effects.allowed[{index}]"
        item = _mapping(value, item_name)
        _require_allowed_keys(item, item_name, ("kind", "path_glob"))
        result.append(
            Effect(
                _enum(EffectKind, item.get("kind"), f"{item_name}.kind"),
                _optional_string(item.get("path_glob"), f"{item_name}.path_glob"),
            )
        )
    return tuple(result)


def _task_metadata(value: object, field_name: str) -> None:
    metadata = _mapping(value, field_name)
    _require_allowed_keys(metadata, field_name, ("workspace_subdir",))
    if "workspace_subdir" in metadata:
        _string(metadata["workspace_subdir"], f"{field_name}.workspace_subdir")


def _task(value: object, field_name: str, *, expected_id: str) -> Task:
    data = _mapping(value, field_name)
    _require_allowed_keys(
        data,
        field_name,
        (
            "id",
            "domain_function",
            "prompts",
            "inputs",
            "outputs",
            "contract",
            "metadata",
            "effort",
        ),
    )
    task_id = _string(data.get("id"), f"{field_name}.id")
    if task_id != expected_id:
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"{field_name}.id must match its task key"
        )
    prompts = tuple(
        _prompt(item, f"{field_name}.prompts[{index}]")
        for index, item in enumerate(
            _sequence(data.get("prompts", ()), f"{field_name}.prompts")
        )
    )
    outputs_data = _mapping(data.get("outputs", {}), f"{field_name}.outputs")
    outputs = tuple(
        _artifact(item, f"{field_name}.outputs.{name}", expected_name=name)
        for name, item in outputs_data.items()
    )
    inputs_data = _mapping(data.get("inputs", {}), f"{field_name}.inputs")
    inputs: list[TaskInput] = []
    for name, item in inputs_data.items():
        item_data = _mapping(item, f"{field_name}.inputs.{name}")
        source: ArtifactRef | ArtifactBinding
        if "source_task_id" in item_data:
            source = _binding(item_data, f"{field_name}.inputs.{name}")
        else:
            source = _artifact_ref(item_data, f"{field_name}.inputs.{name}")
        inputs.append(TaskInput(name, source))
    contract = _mapping(data.get("contract", {}), f"{field_name}.contract")
    _task_metadata(data.get("metadata", {}), f"{field_name}.metadata")
    effort_value = data.get("effort")
    return Task(
        id=task_id,
        domain_function=_string(
            data.get("domain_function"), f"{field_name}.domain_function"
        ),
        prompts=prompts,
        inputs=tuple(inputs),
        outputs=outputs,
        effects=_effects(contract, f"{field_name}.contract"),
        effort=(
            None
            if effort_value is None
            else _enum(Effort, effort_value, f"{field_name}.effort")
        ),
    )


def _reject_constant(token: str) -> None:
    raise ValueError(f"non-standard JSON constant is not allowed: {token}")


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key!r}")
        result[key] = value
    return result


def _decode_payload(payload: Mapping[str, object] | str) -> Mapping[str, object]:
    if isinstance(payload, str):
        if len(payload.encode("utf-8")) > _MAX_LEGACY_JSON_BYTES:
            raise ValidationError(
                "LEGACY_FORMAT_INVALID",
                f"legacy JSON exceeds {_MAX_LEGACY_JSON_BYTES} bytes",
            )
        try:
            decoded = json.loads(
                payload,
                parse_constant=_reject_constant,
                object_pairs_hook=_unique_object,
            )
        except (json.JSONDecodeError, ValueError) as error:
            raise ValidationError(
                "LEGACY_FORMAT_INVALID", "legacy JSON is invalid"
            ) from error
        return _mapping(decoded, "legacy workflow")
    return _mapping(payload, "legacy workflow")


def convert_legacy_workflow(payload: Mapping[str, object] | str) -> Workflow:
    """Convert one prototype v1 workflow payload to the sole v2 model.

    The conversion is deliberately one-way and emits ``DeprecationWarning``.
    The runtime-owned ``metadata.workspace_subdir`` hint is validated and then
    discarded. Every other unsupported field, including semantic metadata and
    legacy artifact constraints, is rejected rather than silently weakened.
    """
    warnings.warn(
        "prototype WorkflowPlan JSON is deprecated; persist clef.workflow/v2 instead",
        DeprecationWarning,
        stacklevel=2,
    )
    data = _decode_payload(payload)
    _require_allowed_keys(
        data,
        "legacy workflow",
        ("schema_version", "id", "tasks", "bindings", "policies", "outputs"),
    )
    schema = data.get("schema_version")
    if schema is not None and schema != "clef.workflow/v1":
        raise ValidationError(
            "LEGACY_FORMAT_INVALID", f"unsupported legacy schema: {schema!r}"
        )
    tasks_data = _mapping(data.get("tasks"), "legacy workflow.tasks")
    tasks = [
        _task(item, f"legacy workflow.tasks.{task_id}", expected_id=task_id)
        for task_id, item in tasks_data.items()
    ]
    positions = {task.id: index for index, task in enumerate(tasks)}
    bindings = _sequence(data.get("bindings", ()), "legacy workflow.bindings")
    for index, value in enumerate(bindings):
        field_name = f"legacy workflow.bindings[{index}]"
        item = _mapping(value, field_name)
        binding = _binding(item, field_name, explicit_edge=True)
        target_task_id = _string(
            item.get("target_task_id"), f"{field_name}.target_task_id"
        )
        input_name = _string(item.get("input_name"), f"{field_name}.input_name")
        if target_task_id not in positions:
            raise ValidationError(
                "LEGACY_FORMAT_INVALID", f"{field_name} targets an unknown task"
            )
        task_position = positions[target_task_id]
        task = tasks[task_position]
        if any(existing.name == input_name for existing in task.inputs):
            continue
        tasks[task_position] = task.add_input(input_name, binding)

    policy_data = _mapping(data.get("policies", {}), "legacy workflow.policies")
    _require_allowed_keys(
        policy_data,
        "legacy workflow.policies",
        ("max_concurrency", "fail_fast", "max_fan_out"),
    )
    policy = WorkflowPolicy(
        max_concurrency=_integer(
            policy_data.get("max_concurrency"),
            "legacy workflow.policies.max_concurrency",
            1,
        ),
        fail_fast=_boolean(
            policy_data.get("fail_fast"), "legacy workflow.policies.fail_fast", True
        ),
        max_fan_out=_integer(
            policy_data.get("max_fan_out"),
            "legacy workflow.policies.max_fan_out",
            32,
        ),
    )
    outputs_data = _mapping(data.get("outputs", {}), "legacy workflow.outputs")
    outputs = tuple(
        WorkflowOutput(name, _binding(item, f"legacy workflow.outputs.{name}"))
        for name, item in outputs_data.items()
    )
    return Workflow(
        id=_string(data.get("id"), "legacy workflow.id"),
        tasks=tuple(tasks),
        outputs=outputs,
        policy=policy,
    )


__all__ = ["convert_legacy_workflow"]
