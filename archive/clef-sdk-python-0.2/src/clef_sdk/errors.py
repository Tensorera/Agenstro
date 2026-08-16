"""Stable public errors for the Clef RPC facade."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from enum import Enum
from itertools import islice

_MAX_FIELD_VIOLATIONS = 128


class RpcCode(str, Enum):
    """Canonical status codes accepted from the generated RPC transport."""

    INVALID_ARGUMENT = "INVALID_ARGUMENT"
    UNAUTHENTICATED = "UNAUTHENTICATED"
    PERMISSION_DENIED = "PERMISSION_DENIED"
    NOT_FOUND = "NOT_FOUND"
    ALREADY_EXISTS = "ALREADY_EXISTS"
    FAILED_PRECONDITION = "FAILED_PRECONDITION"
    ABORTED = "ABORTED"
    RESOURCE_EXHAUSTED = "RESOURCE_EXHAUSTED"
    OUT_OF_RANGE = "OUT_OF_RANGE"
    DEADLINE_EXCEEDED = "DEADLINE_EXCEEDED"
    UNAVAILABLE = "UNAVAILABLE"
    INTERNAL = "INTERNAL"
    DATA_LOSS = "DATA_LOSS"


@dataclass(frozen=True, slots=True)
class FieldViolation:
    """One typed invalid-field detail returned by the daemon."""

    field: str
    description: str

    def __post_init__(self) -> None:
        if not isinstance(self.field, str) or not self.field or len(self.field) > 512:
            raise ValueError("field must be a non-empty string <= 512 characters")
        if (
            not isinstance(self.description, str)
            or not self.description
            or len(self.description) > 4_096
        ):
            raise ValueError(
                "description must be a non-empty string <= 4096 characters"
            )


def _bounded_violations(
    values: Iterable[FieldViolation],
) -> tuple[FieldViolation, ...]:
    result = tuple(islice(values, _MAX_FIELD_VIOLATIONS + 1))
    if len(result) > _MAX_FIELD_VIOLATIONS:
        raise ValueError(
            f"field_violations cannot exceed {_MAX_FIELD_VIOLATIONS} items"
        )
    if not all(isinstance(item, FieldViolation) for item in result):
        raise TypeError("field_violations must contain FieldViolation values")
    return result


class RpcFailure(Exception):
    """Typed failure emitted by a generated-client transport adapter.

    Generated gRPC status objects are converted to this value at the transport
    edge. The SDK never branches on provider or human-readable message text.
    """

    def __init__(
        self,
        status: RpcCode,
        domain_code: str,
        message: str,
        *,
        retryable: bool = False,
        resource_id: str | None = None,
        field_violations: Iterable[FieldViolation] = (),
        correlation_id: str | None = None,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.domain_code = domain_code
        self.message = message
        self.retryable = retryable
        self.resource_id = resource_id
        self.field_violations = _bounded_violations(field_violations)
        self.correlation_id = correlation_id


class ClefError(Exception):
    """Base class for stable SDK failures."""

    def __init__(
        self,
        code: str,
        message: str,
        *,
        retryable: bool = False,
        resource_id: str | None = None,
        field_violations: Iterable[FieldViolation] = (),
        correlation_id: str | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.retryable = retryable
        self.resource_id = resource_id
        self.field_violations = _bounded_violations(field_violations)
        self.correlation_id = correlation_id


class ValidationError(ClefError, ValueError):
    """A local builder or remote request failed validation."""


class AuthenticationError(ClefError):
    """The daemon rejected or could not find client authentication."""


class PermissionDeniedError(ClefError):
    """Authentication succeeded but the granted scope is insufficient."""


class NotFoundError(ClefError):
    """A requested resource is not visible or does not exist."""


class ConflictError(ClefError):
    """An idempotency, revision, or resource-state conflict occurred."""


class CapabilityUnavailableError(ClefError):
    """The server lacks a capability required before an operation starts."""

    def __init__(self, missing: Iterable[str]) -> None:
        bounded = tuple(islice(missing, 257))
        if len(bounded) > 256:
            raise ValueError("missing capabilities cannot exceed 256 items")
        if not all(isinstance(item, str) and item for item in bounded):
            raise TypeError("missing capabilities must be non-empty strings")
        values = tuple(sorted(set(bounded)))
        self.missing = values
        super().__init__(
            "CAPABILITY_UNAVAILABLE",
            f"server is missing required capabilities: {', '.join(values)}",
        )


class ResourceExhaustedError(ClefError):
    """A configured queue, stream, quota, or output limit was reached."""


class SequenceOutOfRangeError(ClefError):
    """A requested event cursor is older than retained history."""


class DeadlineExceededError(ClefError):
    """An RPC did not complete before its explicit deadline."""


class UnavailableError(ClefError):
    """The local daemon or transport is temporarily unavailable."""


class ProtocolError(ClefError):
    """The daemon or adapter stream violated its versioned protocol."""


class DataLossError(ClefError):
    """Durable data or an artifact failed an integrity check."""


class InternalError(ClefError):
    """An internal failure crossed the RPC boundary safely."""


class ClientStateError(ClefError):
    """An SDK operation was attempted before connect or after close."""


_ERROR_TYPES: dict[RpcCode, type[ClefError]] = {
    RpcCode.INVALID_ARGUMENT: ValidationError,
    RpcCode.UNAUTHENTICATED: AuthenticationError,
    RpcCode.PERMISSION_DENIED: PermissionDeniedError,
    RpcCode.NOT_FOUND: NotFoundError,
    RpcCode.ALREADY_EXISTS: ConflictError,
    RpcCode.FAILED_PRECONDITION: ConflictError,
    RpcCode.ABORTED: ConflictError,
    RpcCode.RESOURCE_EXHAUSTED: ResourceExhaustedError,
    RpcCode.OUT_OF_RANGE: SequenceOutOfRangeError,
    RpcCode.DEADLINE_EXCEEDED: DeadlineExceededError,
    RpcCode.UNAVAILABLE: UnavailableError,
    RpcCode.INTERNAL: InternalError,
    RpcCode.DATA_LOSS: DataLossError,
}


def error_from_rpc(failure: RpcFailure) -> ClefError:
    """Map one typed RPC failure without inspecting display text."""
    error_type = _ERROR_TYPES[failure.status]
    return error_type(
        failure.domain_code,
        failure.message,
        retryable=failure.retryable,
        resource_id=failure.resource_id,
        field_violations=failure.field_violations,
        correlation_id=failure.correlation_id,
    )


__all__ = [
    "AuthenticationError",
    "CapabilityUnavailableError",
    "ClefError",
    "ClientStateError",
    "ConflictError",
    "DataLossError",
    "DeadlineExceededError",
    "FieldViolation",
    "InternalError",
    "NotFoundError",
    "PermissionDeniedError",
    "ProtocolError",
    "ResourceExhaustedError",
    "RpcCode",
    "RpcFailure",
    "SequenceOutOfRangeError",
    "UnavailableError",
    "ValidationError",
    "error_from_rpc",
]
