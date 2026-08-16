"""Frozen 0.2 framing shared by Tactus Python workers."""

from __future__ import annotations

import json
import struct
from dataclasses import dataclass
from typing import BinaryIO, cast
from uuid import UUID

from .errors import ProtocolError

PROTOCOL_MAJOR = 1
PROTOCOL_MINOR = 0
MAX_FRAME_BYTES = 256 * 1024
MAX_OUTPUT_CHUNK_BYTES = 64 * 1024

_MAGIC = b"TACT"
_HEADER = struct.Struct(">4sHHI36s")

type JsonValue = (
    bool | int | float | str | list[JsonValue] | dict[str, JsonValue] | None
)
type Message = dict[str, JsonValue]


@dataclass(frozen=True)
class Frame:
    """One validated frame and its execution request identity."""

    request_id: UUID
    message: Message


def encode_frame(
    request_id: UUID,
    message: Message,
    *,
    max_frame_bytes: int = MAX_FRAME_BYTES,
) -> bytes:
    """Encode one canonical JSON payload behind the fixed Tactus header."""
    limit = _validated_limit(max_frame_bytes)
    try:
        payload = json.dumps(
            message,
            allow_nan=False,
            ensure_ascii=True,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    except (TypeError, ValueError) as exc:
        raise ProtocolError("worker message is not bounded JSON") from exc
    if len(payload) > limit:
        raise ProtocolError(f"worker frame exceeds {limit} bytes")
    request_bytes = str(request_id).encode("ascii")
    return (
        _HEADER.pack(
            _MAGIC,
            PROTOCOL_MAJOR,
            PROTOCOL_MINOR,
            len(payload),
            request_bytes,
        )
        + payload
    )


def read_frame(
    stream: BinaryIO,
    *,
    max_frame_bytes: int = MAX_FRAME_BYTES,
) -> Frame | None:
    """Read one frame, returning ``None`` only for clean boundary EOF."""
    limit = _validated_limit(max_frame_bytes)
    header = _read_exact(stream, _HEADER.size, allow_initial_eof=True)
    if header is None:
        return None
    magic, major, minor, payload_length, request_bytes = _HEADER.unpack(header)
    if magic != _MAGIC:
        raise ProtocolError("worker frame magic is invalid")
    if major != PROTOCOL_MAJOR or minor > PROTOCOL_MINOR:
        raise ProtocolError(f"unsupported worker protocol {major}.{minor}")
    if payload_length > limit:
        raise ProtocolError(f"worker frame exceeds {limit} bytes")
    try:
        request_text = request_bytes.decode("ascii")
        request_id = UUID(request_text)
    except (UnicodeDecodeError, ValueError) as exc:
        raise ProtocolError("worker frame request id is invalid") from exc
    if str(request_id) != request_text:
        raise ProtocolError("worker frame request id is not canonical")
    payload = _read_exact(stream, payload_length, allow_initial_eof=False)
    if payload is None:
        raise ProtocolError("worker frame payload is truncated")
    try:
        value = json.loads(
            payload.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_object,
            parse_constant=_reject_json_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as exc:
        raise ProtocolError("worker frame payload is invalid JSON") from exc
    if not isinstance(value, dict):
        raise ProtocolError("worker frame payload must be a JSON object")
    untyped_message = cast(dict[object, object], value)
    if not all(isinstance(key, str) for key in untyped_message):
        raise ProtocolError("worker frame payload must be a JSON object")
    return Frame(request_id=request_id, message=cast(Message, untyped_message))


def write_frame(
    stream: BinaryIO,
    request_id: UUID,
    message: Message,
    *,
    max_frame_bytes: int = MAX_FRAME_BYTES,
) -> None:
    """Write and flush exactly one complete protocol frame."""
    stream.write(encode_frame(request_id, message, max_frame_bytes=max_frame_bytes))
    stream.flush()


def _validated_limit(value: int) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value <= 0
        or value > MAX_FRAME_BYTES
    ):
        raise ProtocolError(
            f"frame limit must be between 1 and {MAX_FRAME_BYTES} bytes"
        )
    return value


def _read_exact(
    stream: BinaryIO,
    size: int,
    *,
    allow_initial_eof: bool,
) -> bytes | None:
    chunks: list[bytes] = []
    remaining = size
    while remaining:
        chunk = stream.read(remaining)
        if not chunk:
            if not chunks and allow_initial_eof:
                return None
            raise ProtocolError("worker frame is truncated")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _reject_json_constant(value: str) -> JsonValue:
    raise ValueError(f"non-finite JSON number: {value}")


def _reject_duplicate_object(
    pairs: list[tuple[str, JsonValue]],
) -> dict[str, JsonValue]:
    value: dict[str, JsonValue] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


__all__ = [
    "MAX_FRAME_BYTES",
    "MAX_OUTPUT_CHUNK_BYTES",
    "PROTOCOL_MAJOR",
    "PROTOCOL_MINOR",
    "Frame",
    "JsonValue",
    "Message",
    "encode_frame",
    "read_frame",
    "write_frame",
]
