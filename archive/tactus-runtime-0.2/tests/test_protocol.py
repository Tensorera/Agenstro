"""Frozen tests for the Tactus 0.2 worker protocol."""

from __future__ import annotations

import io
from uuid import uuid4

import pytest

from tactus_runtime.errors import ProtocolError
from tactus_runtime.protocol import encode_frame, read_frame


class _PartialReader(io.BytesIO):
    def read(self, size: int = -1) -> bytes:
        return super().read(min(size, 3))


def test_frame_round_trip_accepts_partial_reads() -> None:
    request_id = uuid4()
    encoded = encode_frame(request_id, {"type": "Cancel"})

    frame = read_frame(_PartialReader(encoded))

    assert frame is not None
    assert frame.request_id == request_id
    assert frame.message == {"type": "Cancel"}


def test_oversize_length_is_rejected_before_payload_read() -> None:
    encoded = bytearray(encode_frame(uuid4(), {"type": "Cancel"}))
    encoded[8:12] = (4096).to_bytes(4, "big")
    stream = io.BytesIO(encoded)

    with pytest.raises(ProtocolError, match="exceeds 32 bytes"):
        read_frame(stream, max_frame_bytes=32)

    assert stream.tell() == 48


def test_partial_header_and_payload_fail_closed() -> None:
    encoded = encode_frame(uuid4(), {"type": "Cancel"})

    with pytest.raises(ProtocolError, match="truncated"):
        read_frame(io.BytesIO(encoded[:12]))
    with pytest.raises(ProtocolError, match="truncated"):
        read_frame(io.BytesIO(encoded[:-1]))


def test_clean_boundary_eof_is_not_a_truncated_frame() -> None:
    assert read_frame(io.BytesIO()) is None


def test_duplicate_json_keys_fail_closed() -> None:
    encoded = bytearray(encode_frame(uuid4(), {"type": "Cancel"}))
    payload = b'{"type":"Cancel","type":"Execute"}'
    encoded[8:12] = len(payload).to_bytes(4, "big")
    encoded[48:] = payload

    with pytest.raises(ProtocolError, match="invalid JSON"):
        read_frame(io.BytesIO(encoded))
