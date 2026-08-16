"""Frozen 0.2 facade over a generated Tactus daemon transport."""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from types import TracebackType
from typing import Protocol
from uuid import UUID, uuid4

from .errors import ClientError
from .script_file import DirectScript


class SubmissionState(StrEnum):
    """States a daemon may return when accepting one cell."""

    ACCEPTED = "accepted"
    ALREADY_ACCEPTED = "already_accepted"


@dataclass(frozen=True)
class CellSubmission:
    """One immutable cell request passed to the generated RPC adapter."""

    request_id: UUID
    workspace_revision: str
    ordinal: int
    title: str
    source: str
    source_digest: str
    cell_id: UUID | None


@dataclass(frozen=True)
class SubmissionReceipt:
    """Daemon acknowledgement for a durably accepted cell request."""

    request_id: UUID
    run_id: UUID
    state: SubmissionState


class TactusTransport(Protocol):
    """Port implemented by generated gRPC code outside the user model layer."""

    async def submit_cell(self, submission: CellSubmission) -> SubmissionReceipt:
        """Durably accept one idempotent cell request."""
        ...

    async def cancel_run(self, run_id: UUID, request_id: UUID) -> None:
        """Request idempotent cancellation of one daemon-owned run."""
        ...

    async def close(self) -> None:
        """Cancel streams and close the bounded transport."""
        ...


class TactusClient:
    """Small user facade that never owns scheduling or durable state."""

    def __init__(self, transport: TactusTransport, *, max_cells: int = 1024) -> None:
        if (
            not isinstance(max_cells, int)
            or isinstance(max_cells, bool)
            or max_cells <= 0
            or max_cells > 1024
        ):
            raise ClientError("max_cells must be between 1 and 1024")
        self._transport = transport
        self._max_cells = max_cells
        self._closed = False

    async def submit_script(
        self,
        script: DirectScript,
    ) -> tuple[SubmissionReceipt, ...]:
        """Submit executable cells sequentially without an in-process queue."""
        self._ensure_open()
        cells = script.executable_cells
        if len(cells) > self._max_cells:
            raise ClientError(
                f"script has {len(cells)} executable cells; limit is {self._max_cells}"
            )
        receipts: list[SubmissionReceipt] = []
        for cell in cells:
            request = CellSubmission(
                request_id=uuid4(),
                workspace_revision=script.digest,
                ordinal=cell.ordinal,
                title=cell.title,
                source=cell.source,
                source_digest=cell.digest,
                cell_id=cell.cell_id,
            )
            receipt = await self._transport.submit_cell(request)
            if receipt.request_id != request.request_id:
                raise ClientError("daemon receipt request id does not match submission")
            receipts.append(receipt)
        return tuple(receipts)

    async def cancel(self, run_id: UUID) -> None:
        """Forward one explicit cancellation request to the durable authority."""
        self._ensure_open()
        await self._transport.cancel_run(run_id, uuid4())

    async def close(self) -> None:
        """Close the transport exactly once."""
        if self._closed:
            return
        self._closed = True
        await self._transport.close()

    async def __aenter__(self) -> TactusClient:
        self._ensure_open()
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        del exc_type, exc, traceback
        await self.close()

    def _ensure_open(self) -> None:
        if self._closed:
            raise ClientError("Tactus client is closed")


async def submit_main_script(
    client: TactusClient,
    script: DirectScript,
) -> tuple[SubmissionReceipt, ...]:
    """Compatibility submission entry for parsed ``main_script.py`` cells."""
    return await client.submit_script(script)


__all__ = [
    "CellSubmission",
    "SubmissionReceipt",
    "SubmissionState",
    "TactusClient",
    "TactusTransport",
    "submit_main_script",
]
