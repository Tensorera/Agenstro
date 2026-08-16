"""Frozen 0.2 ``jupyter_client`` bridge with reply/idle correlation."""

from __future__ import annotations

import json
import queue
import threading
import time
from collections.abc import Callable, Mapping
from contextlib import suppress
from pathlib import Path
from typing import Protocol, cast

from .protocol import MAX_OUTPUT_CHUNK_BYTES
from .worker import OutputEmitter

MAX_JUPYTER_MESSAGES = 10_000
MAX_MIME_BYTES = 256 * 1024
POLL_SECONDS = 0.02


class JupyterBridgeError(RuntimeError):
    """A kernel lifecycle or bounded-message contract failed."""

    def __init__(self, code: str) -> None:
        super().__init__(code)
        self.code = code


class KernelClientPort(Protocol):
    """Subset of the official blocking client used by the bridge."""

    def execute(
        self,
        code: str,
        *,
        allow_stdin: bool,
        stop_on_error: bool,
    ) -> str:
        """Start execution and return the parent message id."""
        ...

    def get_iopub_msg(self, timeout: float) -> Mapping[str, object]:
        """Read one IOPub message or raise ``queue.Empty``."""
        ...

    def get_shell_msg(self, timeout: float) -> Mapping[str, object]:
        """Read one shell message or raise ``queue.Empty``."""
        ...


class ManagedKernelClientPort(KernelClientPort, Protocol):
    """Blocking client lifecycle used by the owned kernel manager."""

    def start_channels(self) -> None:
        """Start kernel channels."""
        ...

    def wait_for_ready(self, timeout: float) -> None:
        """Wait for kernel readiness within the startup deadline."""
        ...

    def stop_channels(self) -> None:
        """Stop and release all client channels."""
        ...


class KernelManagerPort(Protocol):
    """Owned manager operations used by the optional worker."""

    def start_kernel(self, *, cwd: str) -> None:
        """Start one fresh kernel in the declared workspace."""
        ...

    def blocking_client(self) -> ManagedKernelClientPort:
        """Return the official blocking client."""
        ...

    def is_alive(self) -> bool:
        """Return whether the child kernel still exists."""
        ...

    def interrupt_kernel(self) -> None:
        """Interrupt the child kernel."""
        ...

    def shutdown_kernel(self, *, now: bool) -> None:
        """Shut down the child kernel and owned channels."""
        ...


class JupyterBridge:
    """Correlate one execute request across Jupyter shell and IOPub."""

    def __init__(
        self,
        client: KernelClientPort,
        *,
        is_alive: Callable[[], bool],
        interrupt: Callable[[], None],
        max_messages: int = MAX_JUPYTER_MESSAGES,
        max_mime_bytes: int = MAX_MIME_BYTES,
    ) -> None:
        if max_messages <= 0 or max_messages > MAX_JUPYTER_MESSAGES:
            raise ValueError("max_messages is outside the supported range")
        if max_mime_bytes <= 0 or max_mime_bytes > MAX_MIME_BYTES:
            raise ValueError("max_mime_bytes is outside the supported range")
        self._client = client
        self._is_alive = is_alive
        self._interrupt = interrupt
        self._max_messages = max_messages
        self._max_mime_bytes = max_mime_bytes

    def execute(
        self,
        source: str,
        *,
        emit: OutputEmitter,
        cancellation: threading.Event,
        deadline: float,
    ) -> None:
        """Wait for the matching reply and idle while streaming bounded output."""
        message_id = self._client.execute(
            source,
            allow_stdin=False,
            stop_on_error=True,
        )
        reply_status: str | None = None
        idle = False
        execution_error = False
        message_count = 0
        while reply_status is None or not idle:
            if cancellation.is_set():
                with suppress(Exception):
                    self._interrupt()
                raise JupyterBridgeError("CANCELLED")
            if not self._is_alive():
                raise JupyterBridgeError("KERNEL_DIED")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                if reply_status is None and not idle:
                    raise JupyterBridgeError("MISSING_REPLY_AND_IDLE")
                if reply_status is None:
                    raise JupyterBridgeError("MISSING_REPLY")
                raise JupyterBridgeError("MISSING_IDLE")
            timeout = min(POLL_SECONDS, remaining)
            if reply_status is None:
                shell = self._poll(self._client.get_shell_msg, timeout)
                if shell is not None and _parent_id(shell) == message_id:
                    message_count += 1
                    if _message_type(shell) == "execute_reply":
                        reply_status = _content_text(shell, "status")
            if not idle:
                iopub = self._poll(self._client.get_iopub_msg, timeout)
                if iopub is not None and _parent_id(iopub) == message_id:
                    message_count += 1
                    message_type = _message_type(iopub)
                    if message_type == "status":
                        idle = _content_text(iopub, "execution_state") == "idle"
                    elif message_type == "stream":
                        self._emit_stream(iopub, emit)
                    elif message_type in {"display_data", "execute_result"}:
                        self._emit_display(iopub, emit)
                    elif message_type == "error":
                        execution_error = True
            if message_count > self._max_messages:
                raise JupyterBridgeError("MESSAGE_LIMIT_EXCEEDED")
        if reply_status != "ok" or execution_error:
            raise JupyterBridgeError("KERNEL_EXECUTION_ERROR")

    @staticmethod
    def _poll(
        receive: Callable[[float], Mapping[str, object]],
        timeout: float,
    ) -> Mapping[str, object] | None:
        try:
            return receive(timeout)
        except queue.Empty:
            return None

    def _emit_stream(
        self,
        message: Mapping[str, object],
        emit: OutputEmitter,
    ) -> None:
        content = _content(message)
        name = content.get("name")
        text = content.get("text")
        if (
            not isinstance(name, str)
            or name not in {"stdout", "stderr"}
            or not isinstance(text, str)
        ):
            raise JupyterBridgeError("INVALID_STREAM_MESSAGE")
        payload = text.encode("utf-8", errors="replace")
        if len(payload) > MAX_OUTPUT_CHUNK_BYTES:
            raise JupyterBridgeError("MESSAGE_LIMIT_EXCEEDED")
        emit(name, payload)

    def _emit_display(
        self,
        message: Mapping[str, object],
        emit: OutputEmitter,
    ) -> None:
        data = _content(message).get("data")
        if not isinstance(data, Mapping):
            raise JupyterBridgeError("INVALID_DISPLAY_MESSAGE")
        untyped_data = cast(Mapping[object, object], data)
        if not all(isinstance(key, str) for key in untyped_data):
            raise JupyterBridgeError("INVALID_DISPLAY_MESSAGE")
        typed_data = cast(Mapping[str, object], untyped_data)
        try:
            payload = json.dumps(
                dict(typed_data),
                allow_nan=False,
                ensure_ascii=True,
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
        except (TypeError, ValueError) as exc:
            raise JupyterBridgeError("INVALID_DISPLAY_MESSAGE") from exc
        if len(payload) > self._max_mime_bytes:
            raise JupyterBridgeError("MIME_LIMIT_EXCEEDED")
        emit("display", payload)


class JupyterExecutionEngine:
    """Fresh-kernel execution engine loaded only by the Jupyter worker."""

    def __init__(
        self, *, kernel_name: str = "python3", startup_seconds: float = 30
    ) -> None:
        self._kernel_name = kernel_name
        self._startup_seconds = startup_seconds
        self._manager: KernelManagerPort | None = None
        self._client: ManagedKernelClientPort | None = None
        self._lock = threading.Lock()

    @property
    def capabilities(self) -> tuple[str, ...]:
        """Advertise the optional Jupyter and rich-display capabilities."""
        return ("jupyter", "rich_display")

    def execute(
        self,
        source: str,
        *,
        emit: OutputEmitter,
        cancellation: threading.Event,
        deadline: float,
        filename: str,
    ) -> tuple[str, str | None]:
        """Start one fresh kernel and run a correlated execution."""
        del filename
        try:
            from jupyter_client.manager import KernelManager
        except ImportError:
            return "failed", "CAPABILITY_UNAVAILABLE"
        manager = cast(
            KernelManagerPort,
            KernelManager(kernel_name=self._kernel_name),
        )
        try:
            manager.start_kernel(cwd=str(Path.cwd()))
            client = manager.blocking_client()
            with self._lock:
                self._manager = manager
                self._client = client
            client.start_channels()
            remaining = max(0.001, deadline - time.monotonic())
            client.wait_for_ready(timeout=min(self._startup_seconds, remaining))
            bridge = JupyterBridge(
                client,
                is_alive=manager.is_alive,
                interrupt=manager.interrupt_kernel,
            )
            bridge.execute(
                source,
                emit=emit,
                cancellation=cancellation,
                deadline=deadline,
            )
        except JupyterBridgeError as exc:
            if exc.code == "CANCELLED":
                return "cancelled", None
            return "failed", exc.code
        except Exception:
            return "failed", "KERNEL_START_FAILED"
        finally:
            with suppress(Exception):
                client_value = self._client
                if client_value is not None:
                    client_value.stop_channels()
            with suppress(Exception):
                manager.shutdown_kernel(now=True)
            with self._lock:
                self._manager = None
                self._client = None
        return "completed", None

    def cancel(self) -> None:
        """Interrupt the currently owned kernel when one exists."""
        with self._lock:
            manager = self._manager
        if manager is not None:
            with suppress(Exception):
                manager.interrupt_kernel()

    def close(self) -> None:
        """Force shutdown of the currently owned kernel when one exists."""
        with self._lock:
            manager = self._manager
        if manager is not None:
            with suppress(Exception):
                manager.shutdown_kernel(now=True)


def _parent_id(message: Mapping[str, object]) -> str | None:
    parent = message.get("parent_header")
    if not isinstance(parent, Mapping):
        return None
    untyped_parent = cast(Mapping[object, object], parent)
    if not all(isinstance(key, str) for key in untyped_parent):
        return None
    value = cast(Mapping[str, object], untyped_parent).get("msg_id")
    return value if isinstance(value, str) else None


def _message_type(message: Mapping[str, object]) -> str | None:
    header = message.get("header")
    if not isinstance(header, Mapping):
        return None
    untyped_header = cast(Mapping[object, object], header)
    if not all(isinstance(key, str) for key in untyped_header):
        return None
    value = cast(Mapping[str, object], untyped_header).get("msg_type")
    return value if isinstance(value, str) else None


def _content(message: Mapping[str, object]) -> Mapping[str, object]:
    content = message.get("content")
    if not isinstance(content, Mapping):
        raise JupyterBridgeError("INVALID_KERNEL_MESSAGE")
    untyped_content = cast(Mapping[object, object], content)
    if not all(isinstance(key, str) for key in untyped_content):
        raise JupyterBridgeError("INVALID_KERNEL_MESSAGE")
    return cast(Mapping[str, object], untyped_content)


def _content_text(message: Mapping[str, object], field: str) -> str | None:
    value = _content(message).get(field)
    return value if isinstance(value, str) else None


__all__ = [
    "JupyterBridge",
    "JupyterBridgeError",
    "JupyterExecutionEngine",
    "KernelClientPort",
]
