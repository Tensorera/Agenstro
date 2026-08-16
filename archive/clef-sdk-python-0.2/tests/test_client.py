from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from datetime import UTC, datetime

import pytest

from clef_sdk import (
    Artifact,
    Capability,
    CapabilityUnavailableError,
    Client,
    CompiledWorkflow,
    PermissionDeniedError,
    ProtocolError,
    ResourceExhaustedError,
    RpcCode,
    RpcFailure,
    Run,
    RunEvent,
    RunEventKind,
    RunState,
    ServerInfo,
    Task,
    UnavailableError,
    Workflow,
)


class CloseTrackingSource:
    def __init__(
        self,
        name: str,
        calls: list[str],
        failure: RpcFailure | None = None,
    ) -> None:
        self.name = name
        self.calls = calls
        self.failure = failure

    def __aiter__(self) -> CloseTrackingSource:
        return self

    async def __anext__(self) -> RunEvent:
        await asyncio.Future[None]()
        raise StopAsyncIteration

    async def aclose(self) -> None:
        self.calls.append(self.name)
        if self.failure is not None:
            raise self.failure


class FakeGeneratedRpc:
    def __init__(
        self,
        capabilities: frozenset[str],
        *,
        events: tuple[RunEvent, ...] = (),
    ) -> None:
        self.info = ServerInfo(
            "clef-sdk",
            "0.2.0",
            1,
            0,
            0,
            "daemon-1",
            f"sha256:{'0' * 64}",
            capabilities,
        )
        self.stream_events = events
        self.stream_sources: dict[str, AsyncIterator[RunEvent]] = {}
        self.calls: list[str] = []
        self.cleanup_calls: list[str] = []
        self.closed = False
        self.get_failure: RpcFailure | None = None
        self.close_failure: RpcFailure | None = None

    async def get_server_info(self, *, timeout: float) -> ServerInfo:
        self.calls.append("get_server_info")
        return self.info

    async def compile_workflow(
        self,
        workflow: Workflow,
        *,
        request_id: str,
        timeout: float,
    ) -> CompiledWorkflow:
        self.calls.append("compile_workflow")
        return CompiledWorkflow(workflow.id, "sha256:plan")

    async def start_run(
        self,
        compiled: CompiledWorkflow,
        *,
        workspace_id: str,
        request_id: str,
        timeout: float,
    ) -> Run:
        self.calls.append("start_run")
        return Run("run-1", compiled.workflow_id, RunState.RUNNING)

    async def get_run(self, run_id: str, *, timeout: float) -> Run:
        self.calls.append("get_run")
        if self.get_failure is not None:
            raise self.get_failure
        return Run(run_id, "workflow-1", RunState.RUNNING)

    async def _watch(self) -> AsyncIterator[RunEvent]:
        for event in self.stream_events:
            await asyncio.sleep(0)
            yield event

    def watch_run(
        self,
        run_id: str,
        *,
        after_sequence: int,
        timeout: float,
    ) -> AsyncIterator[RunEvent]:
        self.calls.append("watch_run")
        source = self.stream_sources.get(run_id)
        if source is not None:
            return source
        return self._watch()

    async def cancel_run(
        self,
        run_id: str,
        *,
        request_id: str,
        timeout: float,
    ) -> Run:
        self.calls.append("cancel_run")
        return Run(run_id, "workflow-1", RunState.CANCELLED)

    async def close(self) -> None:
        self.calls.append("close")
        self.cleanup_calls.append("rpc")
        self.closed = True
        if self.close_failure is not None:
            raise self.close_failure


_BASE_CAPABILITIES = frozenset(
    {
        Capability.WORKFLOW_COMPILE.value,
        Capability.RUN_START.value,
        Capability.RUN_GET.value,
        Capability.RUN_WATCH.value,
        Capability.RUN_CANCEL.value,
    }
)


def _workflow(*, require_streaming: bool = False) -> Workflow:
    task = Task.agent("draft", "documents.draft.v1", "Create a draft.").add_output(
        Artifact.text("report", "Draft report", "report.md")
    )
    if require_streaming:
        task = task.require(Capability.STREAMING)
    return Workflow("workflow-1").add(task)


def test_client_submits_and_owns_bounded_terminal_stream() -> None:
    async def scenario() -> None:
        now = datetime.now(UTC)
        rpc = FakeGeneratedRpc(
            _BASE_CAPABILITIES,
            events=(
                RunEvent("run-1", 1, now, RunEventKind.RUN_STARTED),
                RunEvent("run-1", 2, now, RunEventKind.RUN_SUCCEEDED),
            ),
        )
        async with Client(rpc, max_open_streams=2) as client:
            run = await client.submit(
                _workflow(), workspace_id="workspace-1", request_id="submit-1"
            )
            stream = client.events(run.id, buffer_size=2)
            assert stream.buffer_size == 2
            async with stream:
                events = [event async for event in stream]
            assert [event.sequence for event in events] == [1, 2]
            assert stream.closed
        assert rpc.closed
        assert rpc.calls == [
            "get_server_info",
            "compile_workflow",
            "start_run",
            "watch_run",
            "close",
        ]

    asyncio.run(scenario())


def test_required_capability_fails_before_compile_rpc() -> None:
    async def scenario() -> None:
        rpc = FakeGeneratedRpc(_BASE_CAPABILITIES)
        client = Client(rpc)
        await client.connect()
        with pytest.raises(CapabilityUnavailableError) as captured:
            await client.compile(_workflow(require_streaming=True))
        assert captured.value.missing == (Capability.STREAMING.value,)
        assert "compile_workflow" not in rpc.calls
        await client.close()

    asyncio.run(scenario())


def test_context_entry_failure_closes_owned_rpc_channel() -> None:
    async def scenario() -> None:
        rpc = FakeGeneratedRpc(_BASE_CAPABILITIES)
        with pytest.raises(CapabilityUnavailableError):
            async with Client(rpc, required_capabilities=(Capability.STREAMING,)):
                pytest.fail("context body must not run")
        assert rpc.closed

    asyncio.run(scenario())


def test_handshake_rejects_wrong_product_and_disjoint_minor_range() -> None:
    async def scenario() -> None:
        wrong_product = FakeGeneratedRpc(_BASE_CAPABILITIES)
        wrong_product.info = ServerInfo(
            "tactus-runtime",
            "0.2.0",
            1,
            0,
            0,
            "daemon-1",
            f"sha256:{'0' * 64}",
            _BASE_CAPABILITIES,
        )
        with pytest.raises(ProtocolError, match="not clef-sdk"):
            await Client(wrong_product).connect()

        newer_only = FakeGeneratedRpc(_BASE_CAPABILITIES)
        newer_only.info = ServerInfo(
            "clef-sdk",
            "0.3.0",
            1,
            1,
            2,
            "daemon-2",
            f"sha256:{'1' * 64}",
            _BASE_CAPABILITIES,
        )
        with pytest.raises(ProtocolError, match="does not include"):
            await Client(newer_only).connect()

    asyncio.run(scenario())


def test_typed_rpc_status_maps_without_message_parsing() -> None:
    async def scenario() -> None:
        rpc = FakeGeneratedRpc(_BASE_CAPABILITIES)
        rpc.get_failure = RpcFailure(
            RpcCode.PERMISSION_DENIED,
            "WORKSPACE_SCOPE_DENIED",
            "display text can change",
            resource_id="run-1",
        )
        client = Client(rpc)
        await client.connect()
        with pytest.raises(PermissionDeniedError) as captured:
            await client.get_run("run-1")
        assert captured.value.code == "WORKSPACE_SCOPE_DENIED"
        assert captured.value.resource_id == "run-1"
        await client.close()

    asyncio.run(scenario())


@pytest.mark.parametrize("invalid_sequence", [1, 3])
def test_stream_rejects_duplicate_or_gapped_sequence_and_caps_stream_count(
    invalid_sequence: int,
) -> None:
    async def scenario() -> None:
        now = datetime.now(UTC)
        rpc = FakeGeneratedRpc(
            _BASE_CAPABILITIES,
            events=(
                RunEvent("run-1", 1, now, RunEventKind.RUN_STARTED),
                RunEvent("run-1", invalid_sequence, now, RunEventKind.DIAGNOSTIC),
            ),
        )
        client = Client(rpc, max_open_streams=1)
        await client.connect()
        stream = client.events("run-1", buffer_size=1)
        with pytest.raises(ResourceExhaustedError):
            client.events("run-2")
        async with stream:
            assert (await anext(stream)).sequence == 1
            with pytest.raises(ProtocolError, match="not contiguous"):
                await anext(stream)
        await client.close()

    asyncio.run(scenario())


def test_stream_close_failure_is_typed_and_releases_owner_slot() -> None:
    async def scenario() -> None:
        rpc = FakeGeneratedRpc(_BASE_CAPABILITIES)
        rpc.stream_sources["run-1"] = CloseTrackingSource(
            "stream-1",
            rpc.cleanup_calls,
            RpcFailure(
                RpcCode.UNAVAILABLE,
                "STREAM_CLOSE_FAILED",
                "stream close failed",
                retryable=True,
            ),
        )
        client = Client(rpc, max_open_streams=1)
        await client.connect()
        stream = client.events("run-1")
        await stream.__aenter__()
        await asyncio.sleep(0)

        with pytest.raises(UnavailableError) as captured:
            await stream.aclose()
        assert captured.value.code == "STREAM_CLOSE_FAILED"
        assert stream.closed

        replacement = client.events("run-2")
        await replacement.aclose()
        await client.close()
        assert rpc.cleanup_calls == ["stream-1", "rpc"]

    asyncio.run(scenario())


def test_client_close_finishes_cleanup_and_preserves_first_typed_failure() -> None:
    async def scenario() -> None:
        rpc = FakeGeneratedRpc(_BASE_CAPABILITIES)
        rpc.stream_sources["run-1"] = CloseTrackingSource(
            "stream-1",
            rpc.cleanup_calls,
            RpcFailure(
                RpcCode.UNAVAILABLE,
                "FIRST_STREAM_CLOSE_FAILED",
                "first stream close failed",
            ),
        )
        rpc.stream_sources["run-2"] = CloseTrackingSource("stream-2", rpc.cleanup_calls)
        rpc.close_failure = RpcFailure(
            RpcCode.UNAVAILABLE,
            "CHANNEL_CLOSE_FAILED",
            "channel close failed",
        )
        client = Client(rpc, max_open_streams=2)
        await client.connect()
        first = client.events("run-1")
        second = client.events("run-2")
        await first.__aenter__()
        await second.__aenter__()
        await asyncio.sleep(0)

        with pytest.raises(UnavailableError) as captured:
            await client.close()
        assert captured.value.code == "FIRST_STREAM_CLOSE_FAILED"
        assert rpc.cleanup_calls == ["stream-1", "stream-2", "rpc"]
        assert first.closed
        assert second.closed
        assert rpc.closed

        await client.close()
        assert rpc.cleanup_calls == ["stream-1", "stream-2", "rpc"]

    asyncio.run(scenario())
