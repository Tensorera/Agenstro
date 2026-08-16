"""Frozen tests for the Tactus 0.2 worker process."""

from __future__ import annotations

import base64
import hashlib
import os
import subprocess
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import BinaryIO
from uuid import UUID, uuid4

import pytest

from tactus_runtime.protocol import write_frame
from tactus_runtime.worker import WorkerDiedError, WorkerReplyReader


@contextmanager
def _worker(tmp_path: Path) -> Iterator[subprocess.Popen[bytes]]:
    source_root = Path(__file__).parents[1] / "src"
    environment = {
        "PYTHONIOENCODING": "utf-8",
        "PYTHONPATH": str(source_root),
        "PYTHONUTF8": "1",
    }
    for name in ("SYSTEMROOT", "WINDIR"):
        value = os.environ.get(name)
        if value is not None:
            environment[name] = value
    process = subprocess.Popen(
        [sys.executable, "-m", "tactus_runtime.worker"],
        cwd=tmp_path,
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        yield process
    finally:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=5)


def _pipes(
    process: subprocess.Popen[bytes],
) -> tuple[BinaryIO, BinaryIO, BinaryIO]:
    assert process.stdin is not None
    assert process.stdout is not None
    assert process.stderr is not None
    return process.stdin, process.stdout, process.stderr


def _initialize(
    process: subprocess.Popen[bytes],
    tmp_path: Path,
    request_id: UUID,
    *,
    output_bytes: int = 4096,
) -> WorkerReplyReader:
    stdin, stdout, _ = _pipes(process)
    write_frame(
        stdin,
        request_id,
        {
            "type": "Initialize",
            "protocol_major": 1,
            "protocol_minor": 0,
            "workspace": str(tmp_path.resolve()),
            "limits": {
                "output_bytes": output_bytes,
                "log_bytes": 4096,
                "execution_seconds": 10,
            },
        },
    )
    replies = WorkerReplyReader(stdout, request_id)
    assert replies.next_message()["type"] == "Hello"
    ready = replies.next_message()
    assert ready["type"] == "Ready"
    assert ready["capabilities"] == ["script"]
    return replies


def _execute(stdin: BinaryIO, request_id: UUID, source: str) -> None:
    digest = hashlib.sha256(source.encode("utf-8")).hexdigest()
    write_frame(
        stdin,
        request_id,
        {
            "type": "Execute",
            "source": source,
            "source_digest": digest,
            "workspace_revision": "0" * 64,
        },
    )


def _shutdown(
    process: subprocess.Popen[bytes],
    replies: WorkerReplyReader,
    request_id: UUID,
) -> None:
    stdin, stdout, _ = _pipes(process)
    write_frame(stdin, request_id, {"type": "Shutdown"})
    assert replies.next_message()["type"] == "ShutdownComplete"
    stdin.close()
    assert process.wait(timeout=5) == 0
    assert stdout.read() == b""


def test_oversize_output_fails_once_and_stdout_remains_framed(
    tmp_path: Path,
) -> None:
    with _worker(tmp_path) as process:
        request_id = uuid4()
        replies = _initialize(process, tmp_path, request_id, output_bytes=128)
        stdin, _, stderr = _pipes(process)
        _execute(stdin, request_id, 'print("x" * 1024)')

        assert replies.next_message()["type"] == "ExecutionStarted"
        output = replies.next_message()
        terminal = replies.next_message()

        assert output["type"] == "OutputChunk"
        assert len(base64.b64decode(str(output["data"]))) == 128
        assert terminal == {
            "code": "OUTPUT_LIMIT_EXCEEDED",
            "sequence": terminal["sequence"],
            "type": "ExecutionFailed",
        }
        _shutdown(process, replies, request_id)
        assert stderr.read() == b""


def test_cancel_is_acknowledged_and_reaches_one_terminal(tmp_path: Path) -> None:
    with _worker(tmp_path) as process:
        request_id = uuid4()
        replies = _initialize(process, tmp_path, request_id)
        stdin, _, _ = _pipes(process)
        _execute(stdin, request_id, "while True:\n    pass\n")
        assert replies.next_message()["type"] == "ExecutionStarted"

        write_frame(stdin, request_id, {"type": "Cancel"})

        acknowledged = replies.next_message()
        terminal = replies.next_message()
        assert acknowledged["type"] == "CancelAcknowledged"
        assert acknowledged["active"] is True
        assert terminal["type"] == "ExecutionCancelled"
        _shutdown(process, replies, request_id)


def test_worker_death_before_terminal_is_explicit(tmp_path: Path) -> None:
    with _worker(tmp_path) as process:
        request_id = uuid4()
        replies = _initialize(process, tmp_path, request_id)

        process.terminate()
        process.wait(timeout=5)

        with pytest.raises(WorkerDiedError, match="before ShutdownComplete"):
            replies.next_message()
