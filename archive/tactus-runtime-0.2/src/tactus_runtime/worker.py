"""Frozen 0.2 framed Python script worker owned by ``tactusd``."""

from __future__ import annotations

import base64
import contextlib
import hashlib
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path
from types import FrameType
from typing import Any, BinaryIO, Protocol
from uuid import UUID

from .errors import ProtocolError
from .protocol import (
    MAX_FRAME_BYTES,
    MAX_OUTPUT_CHUNK_BYTES,
    PROTOCOL_MAJOR,
    PROTOCOL_MINOR,
    Frame,
    Message,
    read_frame,
    write_frame,
)

DEFAULT_OUTPUT_BYTES = 1024 * 1024
HARD_MAX_OUTPUT_BYTES = 16 * 1024 * 1024
DEFAULT_LOG_BYTES = 64 * 1024
HARD_MAX_LOG_BYTES = 1024 * 1024
DEFAULT_EXECUTION_SECONDS = 300.0
HARD_MAX_EXECUTION_SECONDS = 24 * 60 * 60.0
SHUTDOWN_GRACE_SECONDS = 2.0

_TERMINAL_TYPES = frozenset(
    {"ExecutionCompleted", "ExecutionFailed", "ExecutionCancelled"}
)


class WorkerDiedError(ProtocolError):
    """The worker stream closed before a terminal lifecycle message."""


class _ExecutionCancelled(BaseException):
    pass


class _ExecutionDeadlineExceeded(BaseException):
    pass


class _OutputLimitExceeded(BaseException):
    pass


type OutputEmitter = Callable[[str, bytes], None]
type TraceFunction = Callable[[FrameType, str, object], TraceFunction | None]


class ExecutionEngine(Protocol):
    """One bounded execution capability hosted by the framing process."""

    @property
    def capabilities(self) -> tuple[str, ...]:
        """Return capabilities advertised in ``Ready``."""
        ...

    def execute(
        self,
        source: str,
        *,
        emit: OutputEmitter,
        cancellation: threading.Event,
        deadline: float,
        filename: str,
    ) -> tuple[str, str | None]:
        """Execute once and return terminal kind plus a stable failure code."""
        ...

    def cancel(self) -> None:
        """Propagate cancellation to an optional child runtime."""

    def close(self) -> None:
        """Release child resources before process shutdown."""


class ScriptExecutionEngine:
    """Fresh in-process namespace for one short-lived script execution."""

    @property
    def capabilities(self) -> tuple[str, ...]:
        """Advertise the default non-Jupyter execution capability."""
        return ("script",)

    def execute(
        self,
        source: str,
        *,
        emit: OutputEmitter,
        cancellation: threading.Event,
        deadline: float,
        filename: str,
    ) -> tuple[str, str | None]:
        """Execute source once in a fresh namespace with cooperative limits."""
        stdout = _OutputTextStream("stdout", emit)
        stderr = _OutputTextStream("stderr", emit)

        def trace(frame: FrameType, event: str, argument: object) -> TraceFunction:
            del frame, event, argument
            if cancellation.is_set():
                raise _ExecutionCancelled
            if time.monotonic() >= deadline:
                raise _ExecutionDeadlineExceeded
            return trace

        namespace: dict[str, Any] = {
            "__builtins__": __builtins__,
            "__file__": filename,
            "__name__": "__main__",
            "__package__": None,
        }
        try:
            code = compile(source, filename, "exec")
            sys.settrace(trace)
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                exec(code, namespace, namespace)
        except _ExecutionCancelled:
            return "cancelled", None
        except _ExecutionDeadlineExceeded:
            return "failed", "DEADLINE_EXCEEDED"
        except _OutputLimitExceeded:
            return "failed", "OUTPUT_LIMIT_EXCEEDED"
        except SystemExit:
            return "failed", "SYSTEM_EXIT"
        except BaseException:
            return "failed", "EXECUTION_ERROR"
        finally:
            sys.settrace(None)
        if cancellation.is_set():
            return "cancelled", None
        return "completed", None

    def cancel(self) -> None:
        """Rely on the execution trace observing the shared cancel event."""
        return

    def close(self) -> None:
        """Release no resources; script execution owns no child runtime."""
        return


class WorkerReplyReader:
    """Validate worker replies and make premature process death explicit."""

    def __init__(self, stream: BinaryIO, request_id: UUID) -> None:
        self._stream = stream
        self._request_id = request_id
        self._next_sequence = 1
        self._terminal = False
        self._shutdown = False

    def next_message(self) -> Message:
        """Read the next contiguous reply or raise on worker death."""
        frame = read_frame(self._stream)
        if frame is None:
            if self._shutdown:
                raise EOFError
            raise WorkerDiedError("worker died before ShutdownComplete")
        if frame.request_id != self._request_id:
            raise ProtocolError("worker reply request id does not match")
        message_type = _message_type(frame.message)
        sequence = frame.message.get("sequence")
        if (
            not isinstance(sequence, int)
            or isinstance(sequence, bool)
            or sequence != self._next_sequence
        ):
            raise ProtocolError("worker reply sequence is not contiguous")
        self._next_sequence += 1
        if self._terminal and message_type == "OutputChunk":
            raise ProtocolError("worker emitted output after terminal")
        if message_type in _TERMINAL_TYPES:
            if self._terminal:
                raise ProtocolError("worker emitted more than one terminal")
            self._terminal = True
        if message_type == "ShutdownComplete":
            self._shutdown = True
        return frame.message


class FramedWorkerServer:
    """Strict one-execution lifecycle over stdin/stdout frames."""

    def __init__(self, engine: ExecutionEngine | None = None) -> None:
        self._engine = engine or ScriptExecutionEngine()
        self._request_id: UUID | None = None
        self._state = "await_initialize"
        self._sequence = 0
        self._output_bytes = DEFAULT_OUTPUT_BYTES
        self._execution_seconds = DEFAULT_EXECUTION_SECONDS
        self._cancellation = threading.Event()
        self._execution_thread: threading.Thread | None = None
        self._output_stream: BinaryIO | None = None
        self._write_lock = threading.RLock()
        self._log: _BoundedLog | None = None

    def run(
        self,
        input_stream: BinaryIO,
        output_stream: BinaryIO,
        log_stream: BinaryIO,
    ) -> int:
        """Run until explicit shutdown, protocol failure, or owner EOF."""
        self._output_stream = output_stream
        self._log = _BoundedLog(log_stream, DEFAULT_LOG_BYTES)
        try:
            while True:
                try:
                    frame = read_frame(input_stream)
                except ProtocolError as exc:
                    self._log.write(f"protocol error: {exc}")
                    self._fail_protocol()
                    return 2
                if frame is None:
                    return self._owner_closed()
                try:
                    should_stop = self._handle(frame)
                except ProtocolError as exc:
                    self._log.write(f"protocol error: {exc}")
                    self._fail_protocol()
                    return 2
                if should_stop:
                    return 0
        finally:
            self._cancellation.set()
            self._engine.cancel()
            self._engine.close()

    def _handle(self, frame: Frame) -> bool:
        if self._state == "await_initialize":
            self._initialize(frame)
            return False
        if frame.request_id != self._request_id:
            raise ProtocolError("all worker frames must use one request id")
        message_type = _message_type(frame.message)
        if message_type == "Execute":
            self._start_execution(frame.message)
            return False
        if message_type == "Cancel":
            self._cancel(frame.message)
            return False
        if message_type == "Shutdown":
            _require_fields(frame.message, {"type"})
            self._shutdown()
            return True
        raise ProtocolError(f"unknown worker request: {message_type}")

    def _initialize(self, frame: Frame) -> None:
        message = frame.message
        _require_fields(
            message,
            {"type", "protocol_major", "protocol_minor", "workspace", "limits"},
        )
        if _message_type(message) != "Initialize":
            raise ProtocolError("first worker request must be Initialize")
        if (
            message["protocol_major"] != PROTOCOL_MAJOR
            or message["protocol_minor"] != PROTOCOL_MINOR
        ):
            raise ProtocolError("Initialize protocol version is unsupported")
        workspace = _required_text(message, "workspace", maximum=4096)
        declared_workspace = Path(workspace)
        if not declared_workspace.is_absolute():
            raise ProtocolError("worker workspace must be absolute")
        try:
            matches_workspace = declared_workspace.resolve() == Path.cwd().resolve()
        except OSError as exc:
            raise ProtocolError("worker workspace cannot be resolved") from exc
        if not matches_workspace:
            raise ProtocolError("worker workspace does not match process cwd")
        limits = message["limits"]
        if not isinstance(limits, dict):
            raise ProtocolError("Initialize limits must be an object")
        _require_fields(
            limits,
            {"output_bytes", "log_bytes", "execution_seconds"},
        )
        self._output_bytes = _required_int(
            limits,
            "output_bytes",
            maximum=HARD_MAX_OUTPUT_BYTES,
        )
        log_bytes = _required_int(
            limits,
            "log_bytes",
            maximum=HARD_MAX_LOG_BYTES,
        )
        self._execution_seconds = _required_number(
            limits,
            "execution_seconds",
            maximum=HARD_MAX_EXECUTION_SECONDS,
        )
        if self._log is not None:
            self._log.limit = log_bytes
        self._request_id = frame.request_id
        self._state = "ready"
        self._emit(
            {
                "type": "Hello",
                "protocol_major": PROTOCOL_MAJOR,
                "protocol_minor": PROTOCOL_MINOR,
            }
        )
        self._emit(
            {
                "type": "Ready",
                "capabilities": list(self._engine.capabilities),
                "max_frame_bytes": MAX_FRAME_BYTES,
                "max_output_chunk_bytes": MAX_OUTPUT_CHUNK_BYTES,
            }
        )

    def _start_execution(self, message: Message) -> None:
        if self._state != "ready":
            raise ProtocolError("Execute is only valid after Ready")
        _require_fields(
            message,
            {"type", "source", "source_digest", "workspace_revision"},
        )
        source = _required_text(message, "source", maximum=MAX_FRAME_BYTES)
        source_digest = _required_digest(message, "source_digest")
        _required_digest(message, "workspace_revision")
        if hashlib.sha256(source.encode("utf-8")).hexdigest() != source_digest:
            raise ProtocolError("Execute source digest does not match source")
        self._state = "running"
        self._cancellation.clear()
        self._emit({"type": "ExecutionStarted"})
        thread = threading.Thread(
            target=self._execute,
            args=(source,),
            name="tactus-script-execution",
            daemon=True,
        )
        self._execution_thread = thread
        thread.start()

    def _execute(self, source: str) -> None:
        output = _BoundedOutput(self._output_bytes, self._emit_output)
        request_id = self._request_id
        if request_id is None:
            return
        try:
            terminal, code = self._engine.execute(
                source,
                emit=output.emit,
                cancellation=self._cancellation,
                deadline=time.monotonic() + self._execution_seconds,
                filename=f"<tactus-cell:{request_id}>",
            )
        except BaseException as exc:
            if self._log is not None:
                self._log.write(f"execution engine failed: {type(exc).__name__}")
            terminal, code = "failed", "WORKER_ERROR"
        if output.exhausted:
            terminal, code = "failed", "OUTPUT_LIMIT_EXCEEDED"
        with self._write_lock:
            if self._state != "running":
                return
            if terminal == "completed":
                self._emit_locked({"type": "ExecutionCompleted"})
            elif terminal == "cancelled":
                self._emit_locked({"type": "ExecutionCancelled"})
            else:
                self._emit_locked(
                    {
                        "type": "ExecutionFailed",
                        "code": code or "EXECUTION_ERROR",
                    }
                )
            self._state = "terminal"

    def _cancel(self, message: Message) -> None:
        _require_fields(message, {"type"})
        with self._write_lock:
            active = self._state == "running"
            if active:
                self._cancellation.set()
            self._emit_locked({"type": "CancelAcknowledged", "active": active})
        if active:
            self._engine.cancel()

    def _shutdown(self) -> None:
        thread = self._execution_thread
        if thread is not None and thread.is_alive():
            self._cancellation.set()
            self._engine.cancel()
            thread.join(SHUTDOWN_GRACE_SECONDS)
            if thread.is_alive():
                raise ProtocolError("execution did not stop before shutdown grace")
        self._engine.close()
        self._emit({"type": "ShutdownComplete"})
        self._state = "shutdown"

    def _emit_output(self, stream: str, content: bytes) -> None:
        self._emit(
            {
                "type": "OutputChunk",
                "stream": stream,
                "encoding": "base64",
                "data": base64.b64encode(content).decode("ascii"),
            }
        )

    def _emit(self, message: Message) -> None:
        with self._write_lock:
            self._emit_locked(message)

    def _emit_locked(self, message: Message) -> None:
        if self._request_id is None or self._output_stream is None:
            raise ProtocolError("worker output is not initialized")
        self._sequence += 1
        message = {**message, "sequence": self._sequence}
        write_frame(self._output_stream, self._request_id, message)

    def _fail_protocol(self) -> None:
        if self._request_id is None or self._state in {"terminal", "shutdown"}:
            return
        try:
            self._emit({"type": "ExecutionFailed", "code": "PROTOCOL_ERROR"})
            self._state = "terminal"
        except (OSError, ProtocolError):
            return

    def _owner_closed(self) -> int:
        thread = self._execution_thread
        if thread is None or not thread.is_alive():
            return 0 if self._state in {"terminal", "shutdown"} else 1
        self._cancellation.set()
        self._engine.cancel()
        thread.join(SHUTDOWN_GRACE_SECONDS)
        return 1


class _BoundedOutput:
    def __init__(self, limit: int, emit: OutputEmitter) -> None:
        self._limit = limit
        self._emit = emit
        self._written = 0
        self.exhausted = False

    def emit(self, stream: str, content: bytes) -> None:
        if self.exhausted:
            raise _OutputLimitExceeded
        remaining = self._limit - self._written
        if len(content) > remaining:
            if remaining:
                self._emit_chunks(stream, content[:remaining])
                self._written += remaining
            self.exhausted = True
            raise _OutputLimitExceeded
        self._emit_chunks(stream, content)
        self._written += len(content)

    def _emit_chunks(self, stream: str, content: bytes) -> None:
        for offset in range(0, len(content), MAX_OUTPUT_CHUNK_BYTES):
            self._emit(stream, content[offset : offset + MAX_OUTPUT_CHUNK_BYTES])


class _OutputTextStream:
    def __init__(self, name: str, emit: OutputEmitter) -> None:
        self._name = name
        self._emit = emit

    @property
    def encoding(self) -> str:
        return "utf-8"

    def writable(self) -> bool:
        return True

    def write(self, value: str) -> int:
        if not isinstance(value, str):
            raise TypeError("worker text output must be str")
        self._emit(self._name, value.encode("utf-8", errors="replace"))
        return len(value)

    def flush(self) -> None:
        return


class _BoundedLog:
    def __init__(self, stream: BinaryIO, limit: int) -> None:
        self._stream = stream
        self.limit = limit
        self._written = 0
        self._truncated = False

    def write(self, message: str) -> None:
        payload = (message.rstrip() + "\n").encode("utf-8", errors="replace")
        remaining = max(0, self.limit - self._written)
        if remaining:
            chunk = payload[:remaining]
            self._stream.write(chunk)
            self._stream.flush()
            self._written += len(chunk)
        if len(payload) > remaining and not self._truncated:
            self._truncated = True


def _message_type(message: Message) -> str:
    return _required_text(message, "type", maximum=64)


def _require_fields(message: Message, expected: set[str]) -> None:
    actual = set(message)
    if actual != expected:
        unknown = sorted(actual - expected)
        missing = sorted(expected - actual)
        raise ProtocolError(
            f"message fields differ; missing={missing}, unknown={unknown}"
        )


def _required_text(message: Message, name: str, *, maximum: int) -> str:
    value = message.get(name)
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise ProtocolError(f"{name} must be non-empty text up to {maximum} characters")
    return value


def _required_int(message: Message, name: str, *, maximum: int) -> int:
    value = message.get(name)
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value <= 0
        or value > maximum
    ):
        raise ProtocolError(f"{name} must be between 1 and {maximum}")
    return value


def _required_number(message: Message, name: str, *, maximum: float) -> float:
    value = message.get(name)
    if (
        not isinstance(value, int | float)
        or isinstance(value, bool)
        or value <= 0
        or value > maximum
    ):
        raise ProtocolError(f"{name} must be between 0 and {maximum}")
    return float(value)


def _required_digest(message: Message, name: str) -> str:
    value = _required_text(message, name, maximum=64)
    if len(value) != 64 or any(
        character not in "0123456789abcdef" for character in value
    ):
        raise ProtocolError(f"{name} must be a lowercase SHA-256 digest")
    return value


def main() -> int:
    """Run the script worker without writing human text to stdout."""
    return FramedWorkerServer().run(
        sys.stdin.buffer,
        sys.stdout.buffer,
        sys.stderr.buffer,
    )


if __name__ == "__main__":
    raise SystemExit(main())


__all__ = [
    "ExecutionEngine",
    "FramedWorkerServer",
    "ScriptExecutionEngine",
    "WorkerDiedError",
    "WorkerReplyReader",
    "main",
]
