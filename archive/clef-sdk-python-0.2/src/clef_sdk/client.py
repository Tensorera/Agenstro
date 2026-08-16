"""Async thin client with capability checks and bounded event delivery."""

from __future__ import annotations

import asyncio
import re
import uuid
from collections.abc import AsyncIterator, Awaitable, Callable, Iterable
from contextlib import suppress
from dataclasses import dataclass
from itertools import islice
from typing import Protocol, Self, runtime_checkable

from .errors import (
    CapabilityUnavailableError,
    ClefError,
    ClientStateError,
    InternalError,
    ProtocolError,
    ResourceExhaustedError,
    RpcFailure,
    UnavailableError,
    ValidationError,
    error_from_rpc,
)
from .rpc import GeneratedRpcClient
from .types import (
    Capability,
    CompiledWorkflow,
    Run,
    RunEvent,
    ServerInfo,
    Workflow,
)

_MAX_EVENT_BUFFER = 1_024
_MAX_OPEN_STREAMS = 64
_MAX_CAPABILITIES = 256
_RESOURCE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")


def _deadline(value: float, field_name: str = "timeout") -> float:
    if isinstance(value, bool) or not isinstance(value, int | float) or value <= 0:
        raise ValidationError("INVALID_ARGUMENT", f"{field_name} must be > 0")
    return float(value)


def _request_id(value: str | None) -> str:
    if value is None:
        return f"request-{uuid.uuid4().hex}"
    if not isinstance(value, str) or _RESOURCE_ID.fullmatch(value) is None:
        raise ValidationError(
            "INVALID_ARGUMENT", "request_id has an invalid identifier shape"
        )
    return value


def _resource_id(value: str, field_name: str) -> str:
    if not isinstance(value, str) or _RESOURCE_ID.fullmatch(value) is None:
        raise ValidationError(
            "INVALID_ARGUMENT", f"{field_name} has an invalid identifier shape"
        )
    return value


def _capability_names(
    values: Iterable[Capability | str],
) -> frozenset[str]:
    bounded = tuple(islice(values, _MAX_CAPABILITIES + 1))
    if len(bounded) > _MAX_CAPABILITIES:
        raise ValidationError(
            "INVALID_ARGUMENT",
            f"capabilities cannot exceed {_MAX_CAPABILITIES} items",
        )
    normalized = frozenset(
        item.value if isinstance(item, Capability) else item for item in bounded
    )
    if not all(isinstance(item, str) and item for item in normalized):
        raise ValidationError(
            "INVALID_ARGUMENT", "capabilities must be non-empty strings"
        )
    return normalized


@runtime_checkable
class _ClosableAsyncIterator(Protocol):
    async def aclose(self) -> None:
        """Close the source iterator."""
        ...


@dataclass(frozen=True, slots=True)
class _StreamFailure:
    error: ClefError


@dataclass(frozen=True, slots=True)
class _StreamEnd:
    pass


_END = _StreamEnd()
type _QueueItem = RunEvent | _StreamFailure | _StreamEnd


class EventStream(AsyncIterator[RunEvent]):
    """Single-owner bounded event iterator.

    Use it as an async context manager. The queue applies backpressure to the
    generated server stream; ``aclose`` cancels the sole pump task and closes a
    closable source iterator. The owning :class:`Client` also closes every open
    stream before its channel.
    """

    def __init__(
        self,
        source_factory: Callable[[], AsyncIterator[RunEvent]],
        *,
        run_id: str,
        after_sequence: int,
        buffer_size: int,
        on_close: Callable[[EventStream], None],
    ) -> None:
        if (
            isinstance(buffer_size, bool)
            or not isinstance(buffer_size, int)
            or not 1 <= buffer_size <= _MAX_EVENT_BUFFER
        ):
            raise ValidationError(
                "INVALID_ARGUMENT",
                f"buffer_size must be between 1 and {_MAX_EVENT_BUFFER}",
            )
        if (
            isinstance(after_sequence, bool)
            or not isinstance(after_sequence, int)
            or after_sequence < 0
        ):
            raise ValidationError(
                "INVALID_ARGUMENT", "after_sequence must be an integer >= 0"
            )
        self._source_factory = source_factory
        self._run_id = run_id
        self._last_sequence: int = after_sequence
        self._queue: asyncio.Queue[_QueueItem] = asyncio.Queue(maxsize=buffer_size)
        self._on_close = on_close
        self._producer: asyncio.Task[None] | None = None
        self._source: AsyncIterator[RunEvent] | None = None
        self._closed = False
        self._terminal = False

    def __aiter__(self) -> Self:
        return self

    @property
    def buffer_size(self) -> int:
        """Return the fixed client-side event queue capacity."""
        return self._queue.maxsize

    @property
    def closed(self) -> bool:
        """Return whether this stream has released its source and pump."""
        return self._closed

    async def __aenter__(self) -> Self:
        self._start()
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: object | None,
    ) -> None:
        await self.aclose()

    def _start(self) -> None:
        if self._closed:
            raise ClientStateError("STREAM_CLOSED", "event stream is closed")
        if self._producer is None:
            self._producer = asyncio.create_task(
                self._pump(), name=f"clef-watch-{self._run_id}"
            )

    async def _pump(self) -> None:
        try:
            self._source = self._source_factory()
            async for event in self._source:
                await self._queue.put(event)
        except asyncio.CancelledError:
            return
        except RpcFailure as failure:
            await self._queue.put(_StreamFailure(error_from_rpc(failure)))
        except ClefError as error:
            await self._queue.put(_StreamFailure(error))
        except Exception as error:
            failure = InternalError(
                "RPC_STREAM_FAILED",
                "the generated RPC event stream failed",
                retryable=False,
            )
            failure.__cause__ = error
            await self._queue.put(_StreamFailure(failure))
        finally:
            if not self._closed:
                await self._queue.put(_END)

    async def __anext__(self) -> RunEvent:
        if self._closed:
            raise StopAsyncIteration
        self._start()
        item = await self._queue.get()
        if isinstance(item, _StreamFailure):
            await self.aclose()
            raise item.error
        if isinstance(item, _StreamEnd):
            await self.aclose()
            if not self._terminal:
                raise ProtocolError(
                    "RUN_STREAM_ENDED",
                    "run event stream ended before a terminal event",
                    retryable=True,
                )
            raise StopAsyncIteration
        if item.run_id != self._run_id:
            await self.aclose()
            raise ProtocolError(
                "RUN_EVENT_CORRELATION",
                "run event does not match the requested run",
            )
        if item.sequence != self._last_sequence + 1:
            await self.aclose()
            raise ProtocolError(
                "RUN_EVENT_SEQUENCE",
                "run event sequence is not contiguous",
            )
        if self._terminal:
            await self.aclose()
            raise ProtocolError(
                "RUN_EVENT_AFTER_TERMINAL",
                "run event arrived after the terminal event",
            )
        self._last_sequence = item.sequence
        self._terminal = item.terminal
        if item.terminal:
            await self.aclose()
        return item

    async def aclose(self) -> None:
        """Idempotently cancel the pump and release the generated stream."""
        if self._closed:
            return
        self._closed = True
        try:
            producer = self._producer
            if producer is not None and producer is not asyncio.current_task():
                producer.cancel()
                with suppress(asyncio.CancelledError):
                    await producer
            source = self._source
            if source is not None and isinstance(source, _ClosableAsyncIterator):
                await source.aclose()
        except RpcFailure as failure:
            raise error_from_rpc(failure) from failure
        except ClefError:
            raise
        except Exception as error:
            raise UnavailableError(
                "RPC_STREAM_CLOSE_FAILED",
                "the generated RPC event stream failed to close",
            ) from error
        finally:
            self._on_close(self)


class Client:
    """User-facing async facade over generated Clef RPC clients."""

    def __init__(
        self,
        rpc: GeneratedRpcClient,
        *,
        required_capabilities: Iterable[Capability | str] = (),
        handshake_timeout: float = 5.0,
        max_open_streams: int = 16,
    ) -> None:
        if not isinstance(rpc, GeneratedRpcClient):
            raise TypeError("rpc must implement GeneratedRpcClient")
        if (
            isinstance(max_open_streams, bool)
            or not isinstance(max_open_streams, int)
            or not 1 <= max_open_streams <= _MAX_OPEN_STREAMS
        ):
            raise ValidationError(
                "INVALID_ARGUMENT",
                f"max_open_streams must be between 1 and {_MAX_OPEN_STREAMS}",
            )
        normalized = _capability_names(required_capabilities)
        self._rpc = rpc
        self._required_capabilities = normalized
        self._handshake_timeout = _deadline(handshake_timeout, "handshake_timeout")
        self._max_open_streams = max_open_streams
        self._server_info: ServerInfo | None = None
        self._streams: dict[EventStream, None] = {}
        self._connect_lock = asyncio.Lock()
        self._closed = False

    async def __aenter__(self) -> Self:
        try:
            await self.connect()
            return self
        except BaseException:
            with suppress(ClefError):
                await self.close()
            raise

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: object | None,
    ) -> None:
        await self.close()

    async def _call[T](self, operation: Awaitable[T]) -> T:
        try:
            return await operation
        except RpcFailure as failure:
            raise error_from_rpc(failure) from failure
        except ClefError:
            raise
        except Exception as error:
            raise UnavailableError(
                "RPC_TRANSPORT_ERROR",
                "the generated RPC transport failed",
                retryable=True,
            ) from error

    async def connect(self) -> ServerInfo:
        """Perform one handshake and fail before use on incompatibility."""
        if self._closed:
            raise ClientStateError("CLIENT_CLOSED", "client is closed")
        async with self._connect_lock:
            if self._server_info is not None:
                return self._server_info
            info = await self._call(
                self._rpc.get_server_info(timeout=self._handshake_timeout)
            )
            if info.product != "clef-sdk":
                raise ProtocolError(
                    "SERVER_PRODUCT_MISMATCH",
                    f"server product {info.product!r} is not clef-sdk",
                )
            if not info.supports(1, 0):
                raise ProtocolError(
                    "API_VERSION_UNSUPPORTED",
                    "server API range does not include client API 1.0",
                )
            missing = self._required_capabilities - info.capabilities
            if missing:
                raise CapabilityUnavailableError(missing)
            self._server_info = info
            return info

    @property
    def server_info(self) -> ServerInfo:
        """Return handshake data after connection."""
        self._require_connected()
        assert self._server_info is not None
        return self._server_info

    def _require_connected(self) -> None:
        if self._closed:
            raise ClientStateError("CLIENT_CLOSED", "client is closed")
        if self._server_info is None:
            raise ClientStateError(
                "CLIENT_NOT_CONNECTED", "call await client.connect() before use"
            )

    def require_capabilities(self, capabilities: Iterable[Capability | str]) -> None:
        """Fail locally when negotiated server capabilities are insufficient."""
        self._require_connected()
        assert self._server_info is not None
        required = _capability_names(capabilities)
        missing = required - self._server_info.capabilities
        if missing:
            raise CapabilityUnavailableError(missing)

    async def compile(
        self,
        workflow: Workflow,
        *,
        request_id: str | None = None,
        timeout: float = 10.0,
    ) -> CompiledWorkflow:
        """Submit a workflow definition for deterministic Rust compilation."""
        self._require_connected()
        if not isinstance(workflow, Workflow):
            raise TypeError("workflow must be Workflow")
        self.require_capabilities(
            (Capability.WORKFLOW_COMPILE, *workflow.all_required_capabilities)
        )
        return await self._call(
            self._rpc.compile_workflow(
                workflow,
                request_id=_request_id(request_id),
                timeout=_deadline(timeout),
            )
        )

    async def start(
        self,
        compiled: CompiledWorkflow,
        *,
        workspace_id: str,
        request_id: str | None = None,
        timeout: float = 10.0,
    ) -> Run:
        """Start one durable run from a compiled plan."""
        self._require_connected()
        if not isinstance(compiled, CompiledWorkflow):
            raise TypeError("compiled must be CompiledWorkflow")
        workspace_id = _resource_id(workspace_id, "workspace_id")
        self.require_capabilities((Capability.RUN_START,))
        return await self._call(
            self._rpc.start_run(
                compiled,
                workspace_id=workspace_id,
                request_id=_request_id(request_id),
                timeout=_deadline(timeout),
            )
        )

    async def submit(
        self,
        workflow: Workflow,
        *,
        workspace_id: str,
        request_id: str | None = None,
        timeout: float = 10.0,
    ) -> Run:
        """Compile then start without implementing local scheduling."""
        base_id = _request_id(request_id)
        compiled = await self.compile(
            workflow,
            request_id=f"{base_id[:120]}-compile",
            timeout=timeout,
        )
        return await self.start(
            compiled,
            workspace_id=workspace_id,
            request_id=f"{base_id[:122]}-start",
            timeout=timeout,
        )

    async def get_run(self, run_id: str, *, timeout: float = 5.0) -> Run:
        """Fetch one run snapshot."""
        self._require_connected()
        self.require_capabilities((Capability.RUN_GET,))
        return await self._call(
            self._rpc.get_run(
                _resource_id(run_id, "run_id"), timeout=_deadline(timeout)
            )
        )

    def events(
        self,
        run_id: str,
        *,
        after_sequence: int = 0,
        buffer_size: int = 64,
        timeout: float = 3_600.0,
    ) -> EventStream:
        """Create a bounded, client-owned run event stream."""
        self._require_connected()
        self.require_capabilities((Capability.RUN_WATCH,))
        run_id = _resource_id(run_id, "run_id")
        deadline = _deadline(timeout)
        if len(self._streams) >= self._max_open_streams:
            raise ResourceExhaustedError(
                "CLIENT_STREAM_LIMIT",
                f"client already owns {self._max_open_streams} open streams",
            )
        stream = EventStream(
            lambda: self._rpc.watch_run(
                run_id,
                after_sequence=after_sequence,
                timeout=deadline,
            ),
            run_id=run_id,
            after_sequence=after_sequence,
            buffer_size=buffer_size,
            on_close=self._streams.pop,
        )
        self._streams[stream] = None
        return stream

    async def cancel_run(
        self,
        run_id: str,
        *,
        request_id: str | None = None,
        timeout: float = 10.0,
    ) -> Run:
        """Idempotently request cancellation from Rust authority."""
        self._require_connected()
        self.require_capabilities((Capability.RUN_CANCEL,))
        run_id = _resource_id(run_id, "run_id")
        return await self._call(
            self._rpc.cancel_run(
                run_id,
                request_id=_request_id(request_id),
                timeout=_deadline(timeout),
            )
        )

    async def close(self) -> None:
        """Close all owned streams before closing the generated channel."""
        if self._closed:
            return
        self._closed = True
        first_error: BaseException | None = None
        for stream in tuple(self._streams):
            try:
                await stream.aclose()
            except BaseException as error:
                if first_error is None:
                    first_error = error
        try:
            await self._rpc.close()
        except BaseException as error:
            if isinstance(error, RpcFailure):
                close_error: BaseException = error_from_rpc(error)
                close_error.__cause__ = error
            elif isinstance(error, ClefError) or not isinstance(error, Exception):
                close_error = error
            else:
                close_error = UnavailableError(
                    "RPC_CLOSE_FAILED", "the generated RPC channel failed to close"
                )
                close_error.__cause__ = error
            if first_error is None:
                first_error = close_error
        if first_error is not None:
            raise first_error


__all__ = ["Client", "EventStream"]
