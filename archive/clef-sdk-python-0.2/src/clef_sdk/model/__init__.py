"""Compatibility import path for the thin public request model.

New code should import from :mod:`clef_sdk`. The old class names below are
aliases to the sole v2 builder types; they do not retain a second runtime model.
Use :func:`clef_sdk.convert_legacy_workflow` for persisted v1 dictionaries.
"""

from ..types import (
    Artifact,
    ArtifactBinding,
    ArtifactKind,
    ArtifactRef,
    Capability,
    CompiledWorkflow,
    Effect,
    EffectKind,
    Effort,
    Prompt,
    PromptRole,
    Run,
    RunEvent,
    RunEventKind,
    RunState,
    ServerInfo,
    Task,
    TaskInput,
    Workflow,
    WorkflowOutput,
    WorkflowPolicy,
)

ArtifactSpec = Artifact
SessionTask = Task
WorkflowPlan = Workflow

__all__ = [
    "Artifact",
    "ArtifactBinding",
    "ArtifactKind",
    "ArtifactRef",
    "ArtifactSpec",
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
    "SessionTask",
    "Task",
    "TaskInput",
    "Workflow",
    "WorkflowOutput",
    "WorkflowPlan",
    "WorkflowPolicy",
]
