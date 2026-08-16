"""Typed thin facade over generated Segno RPC transports."""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, runtime_checkable

DEFAULT_DEADLINE_SECONDS = 10.0
DEFAULT_IMPORT_DEADLINE_SECONDS = 120.0
MAX_PACKAGE_BYTES = 64 * 1024 * 1024


class SegnoClientError(RuntimeError):
    """Stable Python error mapped from a canonical RPC status."""

    code = "UNKNOWN"


class InvalidRequestError(SegnoClientError):
    """The request did not satisfy the public contract."""

    code = "INVALID_ARGUMENT"


class AuthenticationError(SegnoClientError):
    """The local session token is absent or invalid."""

    code = "UNAUTHENTICATED"


class PermissionDeniedError(SegnoClientError):
    """The session lacks the required scope."""

    code = "PERMISSION_DENIED"


class NotFoundError(SegnoClientError):
    """The requested Segno resource does not exist."""

    code = "NOT_FOUND"


class ConflictError(SegnoClientError):
    """A resource, request ID, or revision conflicts."""

    code = "ALREADY_EXISTS"


class RevisionConflictError(ConflictError):
    """An optimistic resource revision conflicts."""

    code = "ABORTED"


class PreconditionError(SegnoClientError):
    """The resource state does not permit the operation."""

    code = "FAILED_PRECONDITION"


class CapacityError(SegnoClientError):
    """A bounded daemon quota or queue is exhausted."""

    code = "RESOURCE_EXHAUSTED"


class DeadlineExceededError(SegnoClientError):
    """The RPC did not finish before its deadline."""

    code = "DEADLINE_EXCEEDED"


class DaemonUnavailableError(SegnoClientError):
    """The authenticated local Segno daemon is unavailable."""

    code = "UNAVAILABLE"


class DataLossError(SegnoClientError):
    """The daemon reported invalid persistent data."""

    code = "DATA_LOSS"


class InternalDaemonError(SegnoClientError):
    """The daemon returned an unmapped internal failure."""

    code = "INTERNAL"


_ERROR_TYPES: dict[str, type[SegnoClientError]] = {
    "INVALID_ARGUMENT": InvalidRequestError,
    "UNAUTHENTICATED": AuthenticationError,
    "PERMISSION_DENIED": PermissionDeniedError,
    "NOT_FOUND": NotFoundError,
    "ALREADY_EXISTS": ConflictError,
    "ABORTED": RevisionConflictError,
    "FAILED_PRECONDITION": PreconditionError,
    "RESOURCE_EXHAUSTED": CapacityError,
    "DEADLINE_EXCEEDED": DeadlineExceededError,
    "UNAVAILABLE": DaemonUnavailableError,
    "DATA_LOSS": DataLossError,
    "INTERNAL": InternalDaemonError,
}


@runtime_checkable
class RpcFailure(Protocol):
    """Minimum status surface exposed by grpcio RPC exceptions."""

    def code(self) -> object:
        """Return a canonical status enum."""

    def details(self) -> str:
        """Return display-only error details."""


class SegnoTransport(Protocol):
    """Adapter implemented by generated gRPC bindings and local discovery."""

    def unary(
        self,
        method: str,
        request: Mapping[str, object],
        *,
        timeout: float,
    ) -> Mapping[str, object]:
        """Invoke one unary Segno RPC."""

    def upload(
        self,
        method: str,
        package: Path,
        request: Mapping[str, object],
        *,
        timeout: float,
        max_bytes: int,
    ) -> Mapping[str, object]:
        """Stream one bounded package file to Segno."""


@dataclass(frozen=True, slots=True)
class TaskSummary:
    """Bounded task list item returned by Segno."""

    task_id: str
    revision: int
    enabled: bool
    package_digest: str
    plan_digest: str | None


@dataclass(frozen=True, slots=True)
class TaskPage:
    """One stable cursor page of tasks."""

    tasks: tuple[TaskSummary, ...]
    next_after: str | None


@dataclass(frozen=True, slots=True)
class ImportResult:
    """A newly registered immutable package revision."""

    task_id: str
    revision: int
    package_digest: str
    workflow_spec_digest: str
    enabled: bool


@dataclass(frozen=True, slots=True)
class RunAccepted:
    """A durable manual occurrence accepted by Segno."""

    task_id: str
    occurrence_id: str
    state: str


@dataclass(frozen=True, slots=True)
class OccurrenceStatus:
    """Bounded orchestration status owned by Segno."""

    occurrence_id: str
    task_id: str
    revision: int
    scheduled_for_ms: int
    state: str
    orchestration_run_id: str | None
    summary_code: str | None


def _status_name(value: object) -> str:
    name = getattr(value, "name", None)
    if isinstance(name, str):
        return name
    text = str(value)
    return text.rsplit(".", 1)[-1]


def map_rpc_error(error: BaseException) -> SegnoClientError:
    """Map a grpcio-like failure by status code, never by message text."""

    if not isinstance(error, RpcFailure):
        return InternalDaemonError("Segno RPC transport failed")
    code = _status_name(error.code())
    error_type = _ERROR_TYPES.get(code, InternalDaemonError)
    detail = error.details()
    message = detail if isinstance(detail, str) and detail else f"Segno RPC failed with {code}"
    return error_type(message)


def _expect_record(value: object, location: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise InternalDaemonError(f"invalid {location} response")
    return value


def _expect_string(value: object, location: str, *, nullable: bool = False) -> str | None:
    if nullable and value is None:
        return None
    if not isinstance(value, str) or not value:
        raise InternalDaemonError(f"invalid {location} response field")
    return value


def _expect_int(value: object, location: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise InternalDaemonError(f"invalid {location} response field")
    return value


class SegnoClient:
    """Small typed wrapper around an authenticated generated RPC transport."""

    def __init__(self, transport: SegnoTransport) -> None:
        self._transport = transport

    def _unary(
        self,
        method: str,
        request: Mapping[str, object],
        *,
        timeout: float,
    ) -> Mapping[str, object]:
        if timeout <= 0:
            raise ValueError("timeout must be positive")
        try:
            return self._transport.unary(method, request, timeout=timeout)
        except SegnoClientError:
            raise
        except Exception as error:
            raise map_rpc_error(error) from error

    def list_tasks(
        self,
        *,
        after: str | None = None,
        limit: int = 100,
        timeout: float = DEFAULT_DEADLINE_SECONDS,
    ) -> TaskPage:
        """Read one bounded task page."""

        if isinstance(limit, bool) or not 1 <= limit <= 200:
            raise ValueError("limit must be from 1 through 200")
        raw = self._unary("ListTasks", {"after": after, "limit": limit}, timeout=timeout)
        raw_tasks = raw.get("tasks")
        if not isinstance(raw_tasks, (list, tuple)) or len(raw_tasks) > limit:
            raise InternalDaemonError("invalid task list response")
        tasks: list[TaskSummary] = []
        for value in raw_tasks:
            item = _expect_record(value, "task")
            enabled = item.get("enabled")
            if not isinstance(enabled, bool):
                raise InternalDaemonError("invalid task enabled response field")
            tasks.append(
                TaskSummary(
                    task_id=str(_expect_string(item.get("task_id"), "task_id")),
                    revision=_expect_int(item.get("revision"), "revision"),
                    enabled=enabled,
                    package_digest=str(
                        _expect_string(item.get("package_digest"), "package_digest")
                    ),
                    plan_digest=_expect_string(
                        item.get("plan_digest"), "plan_digest", nullable=True
                    ),
                )
            )
        next_after = _expect_string(raw.get("next_after"), "next_after", nullable=True)
        return TaskPage(tasks=tuple(tasks), next_after=next_after)

    def import_package(
        self,
        package: Path,
        *,
        request_id: str | None = None,
        timeout: float = DEFAULT_IMPORT_DEADLINE_SECONDS,
    ) -> ImportResult:
        """Stream a bounded ZIP to the daemon without loading it into memory."""

        package = package.resolve()
        if timeout <= 0:
            raise ValueError("timeout must be positive")
        if package.suffix.lower() != ".zip" or not package.is_file():
            raise ValueError("package must be an existing .zip file")
        if package.stat().st_size > MAX_PACKAGE_BYTES:
            raise ValueError("package exceeds the 64 MiB upload limit")
        request = {"request_id": request_id or str(uuid.uuid4())}
        try:
            raw = self._transport.upload(
                "ImportPackage",
                package,
                request,
                timeout=timeout,
                max_bytes=MAX_PACKAGE_BYTES,
            )
        except SegnoClientError:
            raise
        except Exception as error:
            raise map_rpc_error(error) from error
        return _parse_import_result(raw)

    def run_now(
        self,
        task_id: str,
        *,
        request_id: str | None = None,
        timeout: float = DEFAULT_DEADLINE_SECONDS,
    ) -> RunAccepted:
        """Create one durable manual occurrence."""

        raw = self._unary(
            "RunTask",
            {"request_id": request_id or str(uuid.uuid4()), "task_id": task_id},
            timeout=timeout,
        )
        return RunAccepted(
            task_id=str(_expect_string(raw.get("task_id"), "task_id")),
            occurrence_id=str(_expect_string(raw.get("occurrence_id"), "occurrence_id")),
            state=str(_expect_string(raw.get("state"), "state")),
        )

    def status(
        self,
        occurrence_id: str,
        *,
        timeout: float = DEFAULT_DEADLINE_SECONDS,
    ) -> OccurrenceStatus:
        """Read one bounded occurrence snapshot."""

        raw = self._unary("GetOccurrence", {"occurrence_id": occurrence_id}, timeout=timeout)
        return OccurrenceStatus(
            occurrence_id=str(_expect_string(raw.get("occurrence_id"), "occurrence_id")),
            task_id=str(_expect_string(raw.get("task_id"), "task_id")),
            revision=_expect_int(raw.get("revision"), "revision"),
            scheduled_for_ms=_expect_int(raw.get("scheduled_for_ms"), "scheduled_for_ms"),
            state=str(_expect_string(raw.get("state"), "state")),
            orchestration_run_id=_expect_string(
                raw.get("orchestration_run_id"), "orchestration_run_id", nullable=True
            ),
            summary_code=_expect_string(raw.get("summary_code"), "summary_code", nullable=True),
        )


def _parse_import_result(raw_value: object) -> ImportResult:
    raw = _expect_record(raw_value, "import")
    enabled = raw.get("enabled")
    if not isinstance(enabled, bool):
        raise InternalDaemonError("invalid enabled response field")
    return ImportResult(
        task_id=str(_expect_string(raw.get("task_id"), "task_id")),
        revision=_expect_int(raw.get("revision"), "revision"),
        package_digest=str(_expect_string(raw.get("package_digest"), "package_digest")),
        workflow_spec_digest=str(
            _expect_string(raw.get("workflow_spec_digest"), "workflow_spec_digest")
        ),
        enabled=enabled,
    )


def connect_local() -> SegnoClient:
    """Fail clearly until generated gRPC discovery bindings are shipped."""

    raise DaemonUnavailableError(
        "authenticated segnod gRPC discovery is not included in this build; "
        "use offline package commands or install matching generated bindings"
    )
