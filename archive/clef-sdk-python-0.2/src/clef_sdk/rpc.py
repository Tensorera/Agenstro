"""Typed boundary implemented around generated Protobuf/gRPC clients."""

from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Protocol, runtime_checkable

from .types import CompiledWorkflow, Run, RunEvent, ServerInfo, Workflow


@runtime_checkable
class GeneratedRpcClient(Protocol):
    """Narrow adapter implemented with generated Python RPC modules.

    This protocol keeps generated module paths and gRPC implementation types out
    of the public facade. Its implementation must convert typed status details to
    :class:`clef_sdk.RpcFailure`; it must not parse status message strings.
    """

    async def get_server_info(self, *, timeout: float) -> ServerInfo:
        """Perform the version and capability handshake."""
        ...

    async def compile_workflow(
        self,
        workflow: Workflow,
        *,
        request_id: str,
        timeout: float,
    ) -> CompiledWorkflow:
        """Compile one workflow through Rust-owned authority."""
        ...

    async def start_run(
        self,
        compiled: CompiledWorkflow,
        *,
        workspace_id: str,
        request_id: str,
        timeout: float,
    ) -> Run:
        """Create one durable run resource."""
        ...

    async def get_run(self, run_id: str, *, timeout: float) -> Run:
        """Read one run snapshot without side effects."""
        ...

    def watch_run(
        self,
        run_id: str,
        *,
        after_sequence: int,
        timeout: float,
    ) -> AsyncIterator[RunEvent]:
        """Open a generated server-streaming RPC."""
        ...

    async def cancel_run(
        self,
        run_id: str,
        *,
        request_id: str,
        timeout: float,
    ) -> Run:
        """Idempotently request cancellation of one run."""
        ...

    async def close(self) -> None:
        """Close the generated channel and its owned resources."""
        ...


__all__ = ["GeneratedRpcClient"]
