from __future__ import annotations

import pytest

from clef_adapter_host import (
    AgentEventKind,
    ImplementationStatus,
    check_transcript,
    discover_manifests,
)
from clef_adapter_host.conformance import TranscriptError


def _transcript(
    adapter_id: str, provider: str, capabilities: list[str]
) -> list[dict[str, object]]:
    base = {
        "type": "event",
        "schema_version": "1.0",
        "session_id": "session-1",
        "turn_id": "turn-1",
        "occurred_at": "2026-08-01T00:00:00Z",
        "provider": provider,
        "payload_bytes": 0,
    }
    return [
        {
            "type": "initialized",
            "adapter_id": adapter_id,
            "provider": provider,
            "protocol_major": 1,
            "protocol_minor": 0,
            "provider_version": "fake-v1",
            "capabilities": capabilities,
            "auth_methods": ["fake"],
        },
        {**base, "event": "session_started", "sequence": 1},
        {**base, "event": "content_delta", "sequence": 2, "payload_bytes": 12},
        {**base, "event": "turn_completed", "sequence": 3},
    ]


def test_all_four_contract_only_manifests_pass_the_same_fake_suite() -> None:
    manifests = discover_manifests()
    assert {manifest.id for manifest in manifests} == {
        "codex-app-server",
        "opencode-acp",
        "generic-acp",
        "claude-sidecar",
    }
    for manifest in manifests:
        assert manifest.implementation_status is ImplementationStatus.CONTRACT_ONLY
        assert manifest.probe_policy == "fake-transcript-only"
        capabilities = sorted(manifest.static_capabilities)[:2]
        result = check_transcript(
            manifest,
            _transcript(manifest.id, manifest.provider, capabilities),
        )
        assert result.adapter_id == manifest.id
        assert result.event_count == 3
        assert result.terminal_event is AgentEventKind.TURN_COMPLETED


def test_provider_raw_fields_cannot_leak_into_normalized_protocol() -> None:
    manifest = discover_manifests()[0]
    transcript = _transcript(manifest.id, manifest.provider, [])
    transcript[1]["provider_raw_payload"] = {"private": True}
    with pytest.raises(TranscriptError, match="unknown fields"):
        check_transcript(manifest, transcript)


def test_post_terminal_event_is_a_protocol_defect() -> None:
    manifest = discover_manifests()[1]
    transcript = _transcript(manifest.id, manifest.provider, [])
    terminal = dict(transcript[-1])
    terminal["event"] = "diagnostic"
    terminal["sequence"] = 4
    transcript.append(terminal)
    with pytest.raises(TranscriptError, match="after the terminal"):
        check_transcript(manifest, transcript)


def test_manifest_capability_is_not_inferred_from_provider_name() -> None:
    manifest = discover_manifests()[2]
    transcript = _transcript(manifest.id, manifest.provider, ["reasoning_effort"])
    with pytest.raises(TranscriptError, match="exceed"):
        check_transcript(manifest, transcript)
