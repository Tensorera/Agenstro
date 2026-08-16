"""Strict immutable builders and result values for the Clef client."""

from __future__ import annotations

import re
from collections.abc import Iterable
from dataclasses import dataclass, field, replace
from datetime import datetime
from enum import Enum
from itertools import islice
from typing import Self

from .errors import ValidationError

_IDENTIFIER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9._-]{0,127}$")
_SLOT = re.compile(r"^[A-Za-z_][A-Za-z0-9_.-]{0,127}$")
_WINDOWS_INVALID = frozenset('<>:"/\\|?*')
_WINDOWS_RESERVED = frozenset(
    {
        "AUX",
        "CON",
        "NUL",
        "PRN",
        *(f"COM{number}" for number in range(1, 10)),
        *(f"LPT{number}" for number in range(1, 10)),
    }
)
_MAX_CAPABILITIES = 64
_MAX_TASKS = 1_024
_MAX_TASK_PORTS = 64
_MAX_PROMPTS = 64
_MAX_EFFECTS = 64
_MAX_WORKFLOW_OUTPUTS = 1_024


def _bounded_tuple[T](
    values: Iterable[T], maximum: int, field_name: str
) -> tuple[T, ...]:
    result = tuple(islice(values, maximum + 1))
    if len(result) > maximum:
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} cannot exceed {maximum} items"
        )
    return result


def _text(value: str, field_name: str, *, maximum: int = 4096) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValidationError("INVALID_ARGUMENT", f"{field_name} must not be empty")
    if len(value) > maximum:
        raise ValidationError(
            "INVALID_ARGUMENT",
            f"{field_name} must contain at most {maximum} characters",
        )
    if any(ord(character) < 32 and character not in "\n\r\t" for character in value):
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} contains a control character"
        )
    return value


def _component(value: str, field_name: str) -> None:
    if value.endswith((" ", ".")):
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} must not end with a space or period"
        )
    if any(character in _WINDOWS_INVALID or ord(character) < 32 for character in value):
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} is not a portable path component"
        )
    if value.partition(".")[0].upper() in _WINDOWS_RESERVED:
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} is reserved on Windows"
        )


def _matched(value: str, field_name: str, pattern: re.Pattern[str]) -> str:
    value = _text(value, field_name, maximum=128)
    _component(value, field_name)
    if pattern.fullmatch(value) is None:
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} has an invalid identifier shape"
        )
    return value


def _relative_path(value: str, field_name: str) -> str:
    value = _text(value, field_name)
    normalized = value.replace("\\", "/")
    if normalized.startswith("/") or re.match(r"^[A-Za-z]:", normalized):
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} must be workspace-relative"
        )
    components = normalized.split("/")
    if any(component in {"", ".", ".."} for component in components):
        raise ValidationError(
            "INVALID_ARGUMENT",
            f"{field_name} must not contain empty or dot components",
        )
    for index, component in enumerate(components):
        _component(component, f"{field_name}[{index}]")
    if components[0].casefold() == ".tactus":
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} cannot use the reserved .tactus root"
        )
    return "/".join(components)


def _enum_value[T: Enum](enum_type: type[T], value: T | str, field_name: str) -> T:
    if isinstance(value, enum_type):
        return value
    if not isinstance(value, str):
        raise ValidationError("INVALID_ARGUMENT", f"{field_name} must be a string")
    try:
        return enum_type(value)
    except ValueError as error:
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} has an unsupported value: {value!r}"
        ) from error


class ArtifactKind(str, Enum):
    """Artifact forms understood by the stable workflow contract."""

    FILE = "file"
    DIRECTORY = "directory"
    TEXT = "text"
    JSON = "json"


class EffectKind(str, Enum):
    """Declared task intent; these values do not grant OS permissions."""

    READ = "read"
    CREATE = "create"
    MODIFY = "modify"
    MOVE = "move"
    DELETE = "delete"
    SHELL = "shell"
    NETWORK = "network"


class Effort(str, Enum):
    """Logical Clef routes, independent of model names and native variants."""

    XHIGH = "xhigh"
    HIGH = "high"
    MEDIUM = "medium"
    LOW = "low"


class PromptRole(str, Enum):
    """Auditable prompt-fragment roles."""

    POLICY = "policy"
    CONTEXT = "context"
    INSTRUCTION = "instruction"
    REPAIR = "repair"


class Capability(str, Enum):
    """Stable daemon and normalized agent capabilities."""

    WORKFLOW_COMPILE = "workflow.compile"
    RUN_START = "run.start"
    RUN_GET = "run.get"
    RUN_WATCH = "run.watch"
    RUN_CANCEL = "run.cancel"
    STREAMING = "agent.streaming"
    SESSIONS = "agent.sessions"
    RESUME = "agent.resume"
    APPROVALS = "agent.approvals"
    STRUCTURED_OUTPUT = "agent.structured-output"
    FILE_CHANGE_EVENTS = "agent.file-change-events"
    USAGE = "agent.usage"
    MODEL_SELECTION = "agent.model-selection"
    REASONING_EFFORT = "agent.reasoning-effort"
    MCP = "agent.mcp"
    SUBAGENTS = "agent.subagents"
    TERMINAL = "agent.terminal"
    COMPACT = "agent.compact"
    TURN_BUDGET = "agent.turn-budget"
    COST_BUDGET = "agent.cost-budget"


def _capabilities(values: Iterable[Capability | str]) -> frozenset[Capability]:
    return frozenset(
        _enum_value(Capability, value, "capability")
        for value in _bounded_tuple(values, _MAX_CAPABILITIES, "capabilities")
    )


def _empty_capabilities() -> frozenset[Capability]:
    return frozenset()


@dataclass(frozen=True, slots=True)
class Prompt:
    """One ordered prompt fragment."""

    role: PromptRole
    content: str
    name: str | None = None
    priority: int = 0

    def __post_init__(self) -> None:
        object.__setattr__(self, "role", _enum_value(PromptRole, self.role, "role"))
        _text(self.content, "content", maximum=1_048_576)
        if self.name is not None:
            _matched(self.name, "name", _SLOT)
        if isinstance(self.priority, bool) or not isinstance(self.priority, int):
            raise ValidationError("INVALID_ARGUMENT", "priority must be an integer")

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {
            "role": self.role.value,
            "content": self.content,
            "name": self.name,
            "priority": self.priority,
        }


@dataclass(frozen=True, slots=True)
class Artifact:
    """A user-friendly declared output artifact builder."""

    name: str
    description: str
    kind: ArtifactKind
    path: str | None = None
    required: bool = True

    def __post_init__(self) -> None:
        _matched(self.name, "name", _SLOT)
        _text(self.description, "description")
        object.__setattr__(self, "kind", _enum_value(ArtifactKind, self.kind, "kind"))
        if self.path is not None:
            object.__setattr__(self, "path", _relative_path(self.path, "path"))
        if not isinstance(self.required, bool):
            raise ValidationError("INVALID_ARGUMENT", "required must be a boolean")

    @classmethod
    def file(cls, name: str, description: str, path: str) -> Self:
        """Declare one required file output."""
        return cls(
            name=name, description=description, kind=ArtifactKind.FILE, path=path
        )

    @classmethod
    def directory(cls, name: str, description: str, path: str) -> Self:
        """Declare one required directory output."""
        return cls(
            name=name,
            description=description,
            kind=ArtifactKind.DIRECTORY,
            path=path,
        )

    @classmethod
    def text(cls, name: str, description: str, path: str) -> Self:
        """Declare one required UTF-8 text output."""
        return cls(
            name=name, description=description, kind=ArtifactKind.TEXT, path=path
        )

    @classmethod
    def json(cls, name: str, description: str, path: str) -> Self:
        """Declare one required JSON output."""
        return cls(
            name=name, description=description, kind=ArtifactKind.JSON, path=path
        )

    def optional(self) -> Self:
        """Return a copy that does not gate successful publication."""
        return replace(self, required=False)

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {
            "name": self.name,
            "description": self.description,
            "kind": self.kind.value,
            "path": self.path,
            "required": self.required,
        }


@dataclass(frozen=True, slots=True)
class ArtifactRef:
    """A concrete immutable artifact input handle owned by a daemon."""

    uri: str
    description: str
    kind: ArtifactKind
    digest: str | None = None
    media_type: str | None = None

    def __post_init__(self) -> None:
        _text(self.uri, "uri")
        _text(self.description, "description")
        object.__setattr__(self, "kind", _enum_value(ArtifactKind, self.kind, "kind"))
        if self.digest is not None:
            _text(self.digest, "digest", maximum=256)
        if self.media_type is not None:
            _text(self.media_type, "media_type", maximum=256)

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {
            "uri": self.uri,
            "description": self.description,
            "kind": self.kind.value,
            "digest": self.digest,
            "media_type": self.media_type,
        }


@dataclass(frozen=True, slots=True)
class ArtifactBinding:
    """A reference to one declared output of an upstream task."""

    source_task_id: str
    output_name: str

    def __post_init__(self) -> None:
        _matched(self.source_task_id, "source_task_id", _IDENTIFIER)
        _matched(self.output_name, "output_name", _SLOT)

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {
            "source_task_id": self.source_task_id,
            "output_name": self.output_name,
        }


@dataclass(frozen=True, slots=True)
class TaskInput:
    """A named task input bound to a concrete or upstream artifact."""

    name: str
    source: ArtifactRef | ArtifactBinding

    def __post_init__(self) -> None:
        _matched(self.name, "name", _SLOT)
        if not isinstance(self.source, ArtifactRef | ArtifactBinding):
            raise ValidationError(
                "INVALID_ARGUMENT", "source must be ArtifactRef or ArtifactBinding"
            )

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {"name": self.name, "source": self.source.to_dict()}


@dataclass(frozen=True, slots=True)
class Effect:
    """One declared effect intent, not an authorization grant."""

    kind: EffectKind
    path_glob: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "kind", _enum_value(EffectKind, self.kind, "kind"))
        if self.path_glob is not None:
            _text(self.path_glob, "path_glob")

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {"kind": self.kind.value, "path_glob": self.path_glob}


@dataclass(frozen=True, slots=True)
class Task:
    """An immutable task builder submitted to Rust-owned workflow authority."""

    id: str
    domain_function: str
    prompts: tuple[Prompt, ...] = ()
    inputs: tuple[TaskInput, ...] = ()
    outputs: tuple[Artifact, ...] = ()
    effects: tuple[Effect, ...] = ()
    required_capabilities: frozenset[Capability] = field(
        default_factory=_empty_capabilities
    )
    preferred_capabilities: frozenset[Capability] = field(
        default_factory=_empty_capabilities
    )
    effort: Effort | None = None

    def __post_init__(self) -> None:
        _matched(self.id, "id", _IDENTIFIER)
        _matched(self.domain_function, "domain_function", _NAME)
        prompts = _bounded_tuple(self.prompts, _MAX_PROMPTS, "prompts")
        inputs = _bounded_tuple(self.inputs, _MAX_TASK_PORTS, "inputs")
        outputs = _bounded_tuple(self.outputs, _MAX_TASK_PORTS, "outputs")
        effects = _bounded_tuple(self.effects, _MAX_EFFECTS, "effects")
        if not all(isinstance(item, Prompt) for item in prompts):
            raise ValidationError(
                "INVALID_ARGUMENT", "prompts must contain Prompt values"
            )
        if not all(isinstance(item, TaskInput) for item in inputs):
            raise ValidationError(
                "INVALID_ARGUMENT", "inputs must contain TaskInput values"
            )
        if not all(isinstance(item, Artifact) for item in outputs):
            raise ValidationError(
                "INVALID_ARGUMENT", "outputs must contain Artifact values"
            )
        if not all(isinstance(item, Effect) for item in effects):
            raise ValidationError(
                "INVALID_ARGUMENT", "effects must contain Effect values"
            )
        for field_name, names in (
            ("inputs", tuple(item.name for item in inputs)),
            ("outputs", tuple(item.name for item in outputs)),
        ):
            if len(names) != len(set(names)):
                raise ValidationError(
                    "INVALID_ARGUMENT", f"{field_name} must have unique names"
                )
        required = _capabilities(self.required_capabilities)
        preferred = _capabilities(self.preferred_capabilities)
        if required & preferred:
            raise ValidationError(
                "INVALID_ARGUMENT",
                "a capability cannot be both required and preferred",
            )
        object.__setattr__(self, "prompts", prompts)
        object.__setattr__(self, "inputs", inputs)
        object.__setattr__(self, "outputs", outputs)
        object.__setattr__(self, "effects", effects)
        object.__setattr__(self, "required_capabilities", required)
        object.__setattr__(self, "preferred_capabilities", preferred)
        if self.effort is not None:
            object.__setattr__(
                self, "effort", _enum_value(Effort, self.effort, "effort")
            )

    @classmethod
    def agent(cls, task_id: str, domain_function: str, instruction: str) -> Self:
        """Create an agent task with one instruction prompt."""
        return cls(
            id=task_id,
            domain_function=domain_function,
            prompts=(Prompt(PromptRole.INSTRUCTION, instruction),),
        )

    def add_prompt(self, prompt: Prompt) -> Self:
        """Return a copy with one appended prompt fragment."""
        return replace(self, prompts=(*self.prompts, prompt))

    def add_input(self, name: str, source: ArtifactRef | ArtifactBinding) -> Self:
        """Return a copy with one named artifact input."""
        return replace(self, inputs=(*self.inputs, TaskInput(name, source)))

    def add_output(self, artifact: Artifact) -> Self:
        """Return a copy with one declared output."""
        return replace(self, outputs=(*self.outputs, artifact))

    def allow(self, kind: EffectKind, path_glob: str | None = None) -> Self:
        """Return a copy with one declared effect intent."""
        return replace(self, effects=(*self.effects, Effect(kind, path_glob)))

    def require(self, *capabilities: Capability) -> Self:
        """Return a copy with additional fail-fast capabilities."""
        return replace(
            self,
            required_capabilities=self.required_capabilities | frozenset(capabilities),
        )

    def prefer(self, *capabilities: Capability) -> Self:
        """Return a copy with additional explicitly degradable capabilities."""
        return replace(
            self,
            preferred_capabilities=self.preferred_capabilities
            | frozenset(capabilities),
        )

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {
            "id": self.id,
            "domain_function": self.domain_function,
            "prompts": [item.to_dict() for item in self.prompts],
            "inputs": [item.to_dict() for item in self.inputs],
            "outputs": [item.to_dict() for item in self.outputs],
            "effects": [item.to_dict() for item in self.effects],
            "required_capabilities": [
                item.value
                for item in sorted(
                    self.required_capabilities, key=lambda item: item.value
                )
            ],
            "preferred_capabilities": [
                item.value
                for item in sorted(
                    self.preferred_capabilities, key=lambda item: item.value
                )
            ],
            "effort": None if self.effort is None else self.effort.value,
        }


@dataclass(frozen=True, slots=True)
class WorkflowOutput:
    """A named workflow output selected from a task output."""

    name: str
    source: ArtifactBinding

    def __post_init__(self) -> None:
        _matched(self.name, "name", _SLOT)
        if not isinstance(self.source, ArtifactBinding):
            raise ValidationError("INVALID_ARGUMENT", "source must be ArtifactBinding")

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {"name": self.name, "source": self.source.to_dict()}


@dataclass(frozen=True, slots=True)
class WorkflowPolicy:
    """Workflow-level admission limits requested from Rust authority."""

    max_concurrency: int = 1
    fail_fast: bool = True
    max_fan_out: int = 32

    def __post_init__(self) -> None:
        for field_name, value in (
            ("max_concurrency", self.max_concurrency),
            ("max_fan_out", self.max_fan_out),
        ):
            if isinstance(value, bool) or not isinstance(value, int) or value < 1:
                raise ValidationError(
                    "INVALID_ARGUMENT", f"{field_name} must be an integer >= 1"
                )
        if not isinstance(self.fail_fast, bool):
            raise ValidationError("INVALID_ARGUMENT", "fail_fast must be a boolean")

    def to_dict(self) -> dict[str, object]:
        """Return the deterministic public request representation."""
        return {
            "max_concurrency": self.max_concurrency,
            "fail_fast": self.fail_fast,
            "max_fan_out": self.max_fan_out,
        }


@dataclass(frozen=True, slots=True)
class Workflow:
    """An immutable ergonomic workflow builder, never a Python scheduler."""

    id: str
    tasks: tuple[Task, ...] = ()
    outputs: tuple[WorkflowOutput, ...] = ()
    policy: WorkflowPolicy = field(default_factory=WorkflowPolicy)
    required_capabilities: frozenset[Capability] = field(
        default_factory=_empty_capabilities
    )

    def __post_init__(self) -> None:
        _matched(self.id, "id", _IDENTIFIER)
        tasks = _bounded_tuple(self.tasks, _MAX_TASKS, "tasks")
        outputs = _bounded_tuple(
            self.outputs, _MAX_WORKFLOW_OUTPUTS, "workflow outputs"
        )
        if not all(isinstance(item, Task) for item in tasks):
            raise ValidationError("INVALID_ARGUMENT", "tasks must contain Task values")
        if not all(isinstance(item, WorkflowOutput) for item in outputs):
            raise ValidationError(
                "INVALID_ARGUMENT", "outputs must contain WorkflowOutput values"
            )
        task_ids = tuple(item.id for item in tasks)
        output_names = tuple(item.name for item in outputs)
        if len(task_ids) != len(set(task_ids)):
            raise ValidationError("INVALID_ARGUMENT", "task IDs must be unique")
        if len(output_names) != len(set(output_names)):
            raise ValidationError("INVALID_ARGUMENT", "workflow outputs must be unique")
        object.__setattr__(self, "tasks", tasks)
        object.__setattr__(self, "outputs", outputs)
        object.__setattr__(
            self, "required_capabilities", _capabilities(self.required_capabilities)
        )
        if not isinstance(self.policy, WorkflowPolicy):
            raise ValidationError("INVALID_ARGUMENT", "policy must be WorkflowPolicy")

    def add(self, *tasks: Task) -> Self:
        """Return a copy with tasks appended in stable declaration order."""
        return replace(self, tasks=(*self.tasks, *tasks))

    def publish(self, name: str, task_id: str, output_name: str) -> Self:
        """Expose one task output as a workflow output."""
        return replace(
            self,
            outputs=(
                *self.outputs,
                WorkflowOutput(name, ArtifactBinding(task_id, output_name)),
            ),
        )

    def require(self, *capabilities: Capability) -> Self:
        """Return a copy with additional workflow-level requirements."""
        return replace(
            self,
            required_capabilities=self.required_capabilities | frozenset(capabilities),
        )

    @property
    def all_required_capabilities(self) -> frozenset[Capability]:
        """Return workflow and task requirements without provider inference."""
        return self.required_capabilities | frozenset(
            capability
            for task in self.tasks
            for capability in task.required_capabilities
        )

    def to_dict(self) -> dict[str, object]:
        """Return the versioned deterministic public request representation."""
        return {
            "schema_version": "clef.workflow/v2",
            "id": self.id,
            "tasks": [task.to_dict() for task in self.tasks],
            "outputs": [output.to_dict() for output in self.outputs],
            "policy": self.policy.to_dict(),
            "required_capabilities": [
                item.value
                for item in sorted(
                    self.required_capabilities, key=lambda item: item.value
                )
            ],
        }


class RunState(str, Enum):
    """Stable public run states."""

    PENDING = "PENDING"
    RUNNING = "RUNNING"
    SUCCEEDED = "SUCCEEDED"
    FAILED = "FAILED"
    CANCELLED = "CANCELLED"


class RunEventKind(str, Enum):
    """Normalized public run events, independent of provider messages."""

    RUN_STARTED = "run_started"
    TASK_CHANGED = "task_changed"
    OUTPUT = "output"
    DIAGNOSTIC = "diagnostic"
    RUN_SUCCEEDED = "run_succeeded"
    RUN_FAILED = "run_failed"
    RUN_CANCELLED = "run_cancelled"


_TERMINAL_EVENTS = frozenset(
    {
        RunEventKind.RUN_SUCCEEDED,
        RunEventKind.RUN_FAILED,
        RunEventKind.RUN_CANCELLED,
    }
)


@dataclass(frozen=True, slots=True)
class ServerInfo:
    """Version and capability handshake returned by the daemon."""

    product: str
    release_version: str
    api_major: int
    api_min_minor: int
    api_max_minor: int
    instance_id: str
    protocol_descriptor: str
    capabilities: frozenset[str]

    def __post_init__(self) -> None:
        for field_name, value in (
            ("api_major", self.api_major),
            ("api_min_minor", self.api_min_minor),
            ("api_max_minor", self.api_max_minor),
        ):
            if isinstance(value, bool) or not isinstance(value, int) or value < 0:
                raise ValidationError(
                    "INVALID_ARGUMENT", f"{field_name} must be an integer >= 0"
                )
        if self.api_min_minor > self.api_max_minor:
            raise ValidationError(
                "INVALID_ARGUMENT", "API minor range must not be reversed"
            )
        _text(self.product, "product", maximum=64)
        _text(self.release_version, "release_version", maximum=64)
        _matched(self.instance_id, "instance_id", _IDENTIFIER)
        _text(self.protocol_descriptor, "protocol_descriptor", maximum=256)
        values = frozenset(
            _bounded_tuple(self.capabilities, 256, "server capabilities")
        )
        if not all(isinstance(item, str) and item for item in values):
            raise ValidationError(
                "INVALID_ARGUMENT", "capabilities must contain non-empty strings"
            )
        object.__setattr__(self, "capabilities", values)

    def supports(self, major: int, minor: int) -> bool:
        """Return whether the server range includes one client API version."""
        return (
            self.api_major == major
            and self.api_min_minor <= minor <= self.api_max_minor
        )


@dataclass(frozen=True, slots=True)
class CompiledWorkflow:
    """A daemon-owned immutable execution-plan reference."""

    workflow_id: str
    plan_digest: str

    def __post_init__(self) -> None:
        _matched(self.workflow_id, "workflow_id", _IDENTIFIER)
        _text(self.plan_digest, "plan_digest", maximum=256)


@dataclass(frozen=True, slots=True)
class Run:
    """A snapshot of a daemon-owned workflow run."""

    id: str
    workflow_id: str
    state: RunState
    last_sequence: int = 0

    def __post_init__(self) -> None:
        _matched(self.id, "id", _IDENTIFIER)
        _matched(self.workflow_id, "workflow_id", _IDENTIFIER)
        object.__setattr__(self, "state", _enum_value(RunState, self.state, "state"))
        if (
            isinstance(self.last_sequence, bool)
            or not isinstance(self.last_sequence, int)
            or self.last_sequence < 0
        ):
            raise ValidationError(
                "INVALID_ARGUMENT", "last_sequence must be an integer >= 0"
            )


@dataclass(frozen=True, slots=True)
class RunEvent:
    """One ordered normalized event from a run watch stream."""

    run_id: str
    sequence: int
    occurred_at: datetime
    kind: RunEventKind
    task_id: str | None = None
    message: str | None = None

    def __post_init__(self) -> None:
        _matched(self.run_id, "run_id", _IDENTIFIER)
        if (
            isinstance(self.sequence, bool)
            or not isinstance(self.sequence, int)
            or self.sequence < 1
        ):
            raise ValidationError(
                "INVALID_ARGUMENT", "sequence must be an integer >= 1"
            )
        if (
            not isinstance(self.occurred_at, datetime)
            or self.occurred_at.tzinfo is None
        ):
            raise ValidationError(
                "INVALID_ARGUMENT", "occurred_at must be timezone-aware"
            )
        object.__setattr__(self, "kind", _enum_value(RunEventKind, self.kind, "kind"))
        if self.task_id is not None:
            _matched(self.task_id, "task_id", _IDENTIFIER)
        if self.message is not None:
            _text(self.message, "message", maximum=16_384)

    @property
    def terminal(self) -> bool:
        """Return whether this event is the run's unique terminal event."""
        return self.kind in _TERMINAL_EVENTS


__all__ = [
    "Artifact",
    "ArtifactBinding",
    "ArtifactKind",
    "ArtifactRef",
    "Capability",
    "CompiledWorkflow",
    "Effect",
    "EffectKind",
    "Effort",
    "Prompt",
    "PromptRole",
    "Run",
    "RunEvent",
    "RunEventKind",
    "RunState",
    "ServerInfo",
    "Task",
    "TaskInput",
    "Workflow",
    "WorkflowOutput",
    "WorkflowPolicy",
]
