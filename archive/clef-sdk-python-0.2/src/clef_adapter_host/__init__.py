"""Contract-only adapter-host manifests and fake transcript checks.

This package does not launch provider processes, read credentials, or perform
live probes. Rust supervision remains the owner of adapter lifecycle.
"""

from .conformance import (
    AgentEventKind,
    ConformanceResult,
    NormalizedEvent,
    check_transcript,
)
from .manifest import (
    AdapterManifest,
    ImplementationStatus,
    discover_manifests,
    load_manifest,
)

__all__ = [
    "AdapterManifest",
    "AgentEventKind",
    "ConformanceResult",
    "ImplementationStatus",
    "NormalizedEvent",
    "check_transcript",
    "discover_manifests",
    "load_manifest",
]
