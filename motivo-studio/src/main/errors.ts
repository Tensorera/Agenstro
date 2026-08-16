import type { ServiceError } from "@grpc/grpc-js";
import { status } from "@grpc/grpc-js";
import type { StudioError } from "../shared/contracts";

export class StudioFault extends Error {
  readonly detail: StudioError;

  constructor(detail: StudioError) {
    super(detail.message);
    this.name = "StudioFault";
    this.detail = detail;
  }
}

export function unavailable(message: string): StudioFault {
  return new StudioFault({
    code: "DAEMON_UNAVAILABLE",
    category: "connection",
    retryable: true,
    message,
    userAction: "Install matching daemons or inspect the local diagnostic log.",
  });
}

export function grpcFault(error: ServiceError): StudioFault {
  const mapping: Partial<Record<number, Pick<StudioError, "category" | "retryable">>> = {
    [status.INVALID_ARGUMENT]: { category: "validation", retryable: false },
    [status.ALREADY_EXISTS]: { category: "conflict", retryable: false },
    [status.FAILED_PRECONDITION]: { category: "conflict", retryable: false },
    [status.ABORTED]: { category: "conflict", retryable: true },
    [status.RESOURCE_EXHAUSTED]: { category: "resource", retryable: true },
    [status.UNAVAILABLE]: { category: "connection", retryable: true },
    [status.DEADLINE_EXCEEDED]: { category: "connection", retryable: true },
  };
  const selected = mapping[error.code] ?? { category: "internal" as const, retryable: false };
  return new StudioFault({
    code: grpcCode(error.code),
    category: selected.category,
    retryable: selected.retryable,
    message: safeGrpcMessage(error.code),
  });
}

function grpcCode(code: number): string {
  const label = status[code];
  return typeof label === "string" ? `RPC_${label}` : "RPC_UNKNOWN";
}

function safeGrpcMessage(code: number): string {
  switch (code) {
    case status.INVALID_ARGUMENT:
      return "The daemon rejected an invalid request.";
    case status.ALREADY_EXISTS:
    case status.FAILED_PRECONDITION:
    case status.ABORTED:
      return "The operation conflicts with newer daemon state.";
    case status.RESOURCE_EXHAUSTED:
      return "A daemon resource limit was reached.";
    case status.UNAVAILABLE:
    case status.DEADLINE_EXCEEDED:
      return "The daemon is temporarily unavailable.";
    default:
      return "The daemon request failed.";
  }
}

export function normalizeFault(error: unknown): StudioError {
  if (error instanceof StudioFault) {
    return error.detail;
  }
  return {
    code: "INTERNAL_ERROR",
    category: "internal",
    retryable: false,
    message: "Motivo Studio could not complete the operation.",
  };
}
