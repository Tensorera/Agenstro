"""Frozen tests for the Tactus 0.2 Jupyter bridge."""

from __future__ import annotations

import queue
import threading
import time
from collections import deque
from collections.abc import Mapping

import pytest

from tactus_runtime.jupyter_bridge import JupyterBridge, JupyterBridgeError


class _Client:
    def __init__(
        self,
        *,
        shell: list[Mapping[str, object] | None],
        iopub: list[Mapping[str, object] | None],
    ) -> None:
        self.shell = deque(shell)
        self.iopub = deque(iopub)

    def execute(
        self,
        code: str,
        *,
        allow_stdin: bool,
        stop_on_error: bool,
    ) -> str:
        assert code
        assert not allow_stdin
        assert stop_on_error
        return "parent"

    def get_shell_msg(self, timeout: float) -> Mapping[str, object]:
        return self._next(self.shell, timeout)

    def get_iopub_msg(self, timeout: float) -> Mapping[str, object]:
        return self._next(self.iopub, timeout)

    @staticmethod
    def _next(
        messages: deque[Mapping[str, object] | None],
        timeout: float,
    ) -> Mapping[str, object]:
        del timeout
        if not messages:
            raise queue.Empty
        message = messages.popleft()
        if message is None:
            raise queue.Empty
        return message


def _message(message_type: str, content: Mapping[str, object]) -> Mapping[str, object]:
    return {
        "header": {"msg_type": message_type},
        "parent_header": {"msg_id": "parent"},
        "content": dict(content),
    }


def _run(
    client: _Client,
    *,
    alive: bool = True,
    timeout: float = 0.05,
    max_mime_bytes: int = 1024,
) -> list[tuple[str, bytes]]:
    output: list[tuple[str, bytes]] = []
    JupyterBridge(
        client,
        is_alive=lambda: alive,
        interrupt=lambda: None,
        max_mime_bytes=max_mime_bytes,
    ).execute(
        "print('ok')",
        emit=lambda stream, content: output.append((stream, content)),
        cancellation=threading.Event(),
        deadline=time.monotonic() + timeout,
    )
    return output


def test_iopub_can_arrive_before_reply_and_both_reply_idle_are_required() -> None:
    client = _Client(
        shell=[None, _message("execute_reply", {"status": "ok"})],
        iopub=[
            _message("stream", {"name": "stdout", "text": "ok\n"}),
            _message("status", {"execution_state": "idle"}),
        ],
    )

    assert _run(client) == [("stdout", b"ok\n")]


@pytest.mark.parametrize(
    ("shell", "iopub", "code"),
    [
        ([_message("execute_reply", {"status": "ok"})], [], "MISSING_IDLE"),
        ([], [_message("status", {"execution_state": "idle"})], "MISSING_REPLY"),
    ],
)
def test_missing_reply_or_idle_has_a_distinct_terminal_code(
    shell: list[Mapping[str, object] | None],
    iopub: list[Mapping[str, object] | None],
    code: str,
) -> None:
    with pytest.raises(JupyterBridgeError, match=code):
        _run(_Client(shell=shell, iopub=iopub), timeout=0.01)


def test_kernel_death_is_not_reported_as_timeout() -> None:
    with pytest.raises(JupyterBridgeError, match="KERNEL_DIED"):
        _run(_Client(shell=[], iopub=[]), alive=False)


def test_oversize_mime_bundle_is_rejected() -> None:
    client = _Client(
        shell=[_message("execute_reply", {"status": "ok"})],
        iopub=[_message("display_data", {"data": {"text/plain": "x" * 100}})],
    )

    with pytest.raises(JupyterBridgeError, match="MIME_LIMIT_EXCEEDED"):
        _run(client, max_mime_bytes=16)


def test_cancel_interrupts_before_waiting_for_reply_or_idle() -> None:
    cancelled = threading.Event()
    cancelled.set()
    interrupted: list[bool] = []
    bridge = JupyterBridge(
        _Client(shell=[], iopub=[]),
        is_alive=lambda: True,
        interrupt=lambda: interrupted.append(True),
    )

    with pytest.raises(JupyterBridgeError, match="CANCELLED"):
        bridge.execute(
            "while True: pass",
            emit=lambda stream, content: None,
            cancellation=cancelled,
            deadline=time.monotonic() + 1,
        )

    assert interrupted == [True]
