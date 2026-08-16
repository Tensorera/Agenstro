"""Stable exceptions for the thin Tactus Python surface."""

from __future__ import annotations


class TactusRuntimeError(RuntimeError):
    """Base error for public Tactus Python operations."""


class ProtocolError(TactusRuntimeError):
    """A worker frame or message violated the negotiated contract."""


class ScriptError(TactusRuntimeError):
    """A main script could not be parsed or submitted safely."""


class ClientError(TactusRuntimeError):
    """The thin daemon client could not complete an operation."""


class WorkspaceError(TactusRuntimeError):
    """A project-local Tactus workspace is missing or unusable."""


class ConfigurationError(TactusRuntimeError):
    """The Tactus TOML or normalized runtime configuration is invalid."""


class ToolError(TactusRuntimeError):
    """A required Haskell command-line tool is missing or cannot start."""


class PluginProtocolError(TactusRuntimeError):
    """A provider or effect plugin violated the one-shot JSONL protocol."""


__all__ = [
    "ClientError",
    "ConfigurationError",
    "PluginProtocolError",
    "ProtocolError",
    "ScriptError",
    "TactusRuntimeError",
    "ToolError",
    "WorkspaceError",
]
