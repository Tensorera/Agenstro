"""Frozen tests for the Tactus 0.2 script client."""

from __future__ import annotations

import asyncio
from pathlib import Path
from uuid import UUID, uuid4

import pytest

from tactus_runtime.client import (
    CellSubmission,
    SubmissionReceipt,
    SubmissionState,
    TactusClient,
)
from tactus_runtime.errors import ScriptError
from tactus_runtime.script_file import parse_direct_script


class _Transport:
    def __init__(self) -> None:
        self.submissions: list[CellSubmission] = []
        self.cancelled: list[UUID] = []
        self.closed = False

    async def submit_cell(self, submission: CellSubmission) -> SubmissionReceipt:
        self.submissions.append(submission)
        return SubmissionReceipt(
            request_id=submission.request_id,
            run_id=uuid4(),
            state=SubmissionState.ACCEPTED,
        )

    async def cancel_run(self, run_id: UUID, request_id: UUID) -> None:
        assert request_id
        self.cancelled.append(run_id)

    async def close(self) -> None:
        self.closed = True


def test_parser_preserves_stable_markers_and_ignores_string_text(
    tmp_path: Path,
) -> None:
    cell_id = uuid4()
    source = (
        f"# %% [tactus-cell:{cell_id}] Build\n"
        'text = """first\n'
        "# %% not a marker\n"
        'last"""\n'
        "# %% Validate\n"
        "assert 'not a marker' in text\n"
    )

    script = parse_direct_script(source, path=tmp_path / "main_script.py")

    assert len(script.cells) == 2
    assert script.cells[0].cell_id == cell_id
    assert script.cells[0].title == "Build"
    assert "# %% not a marker" in script.cells[0].source
    assert script.cells[1].title == "Validate"


def test_duplicate_stable_marker_is_rejected(tmp_path: Path) -> None:
    cell_id = uuid4()
    source = (
        f"# %% [tactus-cell:{cell_id}] One\nvalue = 1\n"
        f"# %% [tactus-cell:{cell_id}] Two\nvalue = 2\n"
    )

    with pytest.raises(ScriptError, match="duplicate stable cell id"):
        parse_direct_script(source, path=tmp_path / "main_script.py")


def test_client_submits_cells_sequentially_and_forwards_cancel(
    tmp_path: Path,
) -> None:
    script = parse_direct_script(
        "# %% One\nvalue = 1\n# %% Two\nvalue = 2\n",
        path=tmp_path / "main_script.py",
    )
    transport = _Transport()
    client = TactusClient(transport)

    receipts = asyncio.run(client.submit_script(script))
    run_id = receipts[0].run_id
    asyncio.run(client.cancel(run_id))
    asyncio.run(client.close())

    assert len(receipts) == 2
    assert [item.ordinal for item in transport.submissions] == [1, 2]
    assert all(
        item.workspace_revision == script.digest for item in transport.submissions
    )
    assert transport.cancelled == [run_id]
    assert transport.closed
