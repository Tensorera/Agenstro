"""Strict, side-effect-free adapter manifest discovery."""

from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from enum import Enum
from importlib.resources import files
from typing import cast

_MANIFEST_NAMES = (
    "claude-sidecar.json",
    "codex-app-server.json",
    "generic-acp.json",
    "opencode-acp.json",
)
_MAX_CAPABILITIES = 64


class ManifestError(ValueError):
    """A bundled adapter manifest is malformed or unsupported."""


class ImplementationStatus(str, Enum):
    """Whether a manifest describes executable code or contract evidence."""

    CONTRACT_ONLY = "contract-only"


def _mapping(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise ManifestError(f"{field_name} must be an object")
    untyped = cast(Mapping[object, object], value)
    if not all(isinstance(key, str) for key in untyped):
        raise ManifestError(f"{field_name} keys must be strings")
    return cast(Mapping[str, object], untyped)


def _sequence(value: object, field_name: str) -> Sequence[object]:
    if isinstance(value, str | bytes) or not isinstance(value, Sequence):
        raise ManifestError(f"{field_name} must be an array")
    return cast(Sequence[object], value)


def _string(value: object, field_name: str, *, maximum: int = 256) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ManifestError(f"{field_name} must be a non-empty string")
    if len(value) > maximum or any(ord(character) < 32 for character in value):
        raise ManifestError(f"{field_name} is too long or contains control characters")
    return value


def _integer(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ManifestError(f"{field_name} must be an integer >= 0")
    return value


def _strings(value: object, field_name: str, *, maximum: int) -> tuple[str, ...]:
    sequence = _sequence(value, field_name)
    if len(sequence) > maximum:
        raise ManifestError(f"{field_name} contains more than {maximum} items")
    result = tuple(
        _string(item, f"{field_name}[{index}]") for index, item in enumerate(sequence)
    )
    if len(result) != len(set(result)):
        raise ManifestError(f"{field_name} must not contain duplicates")
    return result


@dataclass(frozen=True, slots=True)
class AdapterManifest:
    """A discoverable adapter package boundary without runtime side effects."""

    schema_version: str
    id: str
    adapter_version: str
    provider: str
    protocol_major: int
    protocol_minor: int
    tested_provider_versions: tuple[str, ...]
    supported_platforms: tuple[str, ...]
    static_capabilities: frozenset[str]
    command: tuple[str, ...]
    implementation_status: ImplementationStatus
    probe_policy: str

    @classmethod
    def from_dict(cls, value: Mapping[str, object]) -> AdapterManifest:
        """Decode one strict manifest mapping."""
        data = _mapping(value, "manifest")
        expected = {
            "schema_version",
            "id",
            "adapter_version",
            "provider",
            "protocol",
            "tested_provider_versions",
            "supported_platforms",
            "static_capabilities",
            "command",
            "implementation_status",
            "probe_policy",
        }
        unknown = set(data) - expected
        missing = expected - set(data)
        if unknown or missing:
            raise ManifestError(
                f"manifest keys differ: missing={sorted(missing)}, unknown={sorted(unknown)}"
            )
        protocol = _mapping(data["protocol"], "manifest.protocol")
        if set(protocol) != {"major", "minor"}:
            raise ManifestError(
                "manifest.protocol must contain exactly major and minor"
            )
        capabilities = _strings(
            data["static_capabilities"],
            "manifest.static_capabilities",
            maximum=_MAX_CAPABILITIES,
        )
        try:
            status = ImplementationStatus(
                _string(data["implementation_status"], "manifest.implementation_status")
            )
        except ValueError as error:
            raise ManifestError("unsupported implementation_status") from error
        manifest = cls(
            schema_version=_string(data["schema_version"], "manifest.schema_version"),
            id=_string(data["id"], "manifest.id"),
            adapter_version=_string(
                data["adapter_version"], "manifest.adapter_version"
            ),
            provider=_string(data["provider"], "manifest.provider"),
            protocol_major=_integer(protocol["major"], "manifest.protocol.major"),
            protocol_minor=_integer(protocol["minor"], "manifest.protocol.minor"),
            tested_provider_versions=_strings(
                data["tested_provider_versions"],
                "manifest.tested_provider_versions",
                maximum=32,
            ),
            supported_platforms=_strings(
                data["supported_platforms"],
                "manifest.supported_platforms",
                maximum=16,
            ),
            static_capabilities=frozenset(capabilities),
            command=_strings(data["command"], "manifest.command", maximum=16),
            implementation_status=status,
            probe_policy=_string(data["probe_policy"], "manifest.probe_policy"),
        )
        if manifest.schema_version != "agentro.adapter-manifest/v1":
            raise ManifestError("manifest schema_version is unsupported")
        if manifest.protocol_major != 1:
            raise ManifestError("manifest protocol major is unsupported")
        if manifest.implementation_status is not ImplementationStatus.CONTRACT_ONLY:
            raise ManifestError("this package accepts contract-only manifests")
        if manifest.probe_policy != "fake-transcript-only":
            raise ManifestError("this package forbids live probe policies")
        return manifest


def load_manifest(name: str) -> AdapterManifest:
    """Load one bundled manifest without process, network, or home access."""
    if name not in _MANIFEST_NAMES:
        raise ManifestError(f"unknown bundled adapter manifest: {name!r}")
    resource = files("clef_adapter_host.manifests").joinpath(name)
    try:
        payload = resource.read_text(encoding="utf-8")
        decoded = json.loads(payload)
    except (OSError, json.JSONDecodeError) as error:
        raise ManifestError(f"cannot decode bundled manifest {name!r}") from error
    return AdapterManifest.from_dict(_mapping(decoded, name))


def discover_manifests() -> tuple[AdapterManifest, ...]:
    """Return the fixed, bounded set of bundled contract manifests."""
    return tuple(load_manifest(name) for name in _MANIFEST_NAMES)


__all__ = [
    "AdapterManifest",
    "ImplementationStatus",
    "ManifestError",
    "discover_manifests",
    "load_manifest",
]
