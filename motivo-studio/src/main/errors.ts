import type { StudioError } from "../shared/contracts";

export class MainProcessError extends Error {
  readonly detail: StudioError;

  constructor(detail: StudioError) {
    super(detail.message);
    this.name = "MainProcessError";
    this.detail = detail;
  }
}

export function asStudioError(error: unknown): StudioError {
  if (error instanceof MainProcessError) return error.detail;
  return {
    code: "internal_error",
    category: "internal",
    retryable: false,
    message: "Unexpected Motivo Studio failure.",
  };
}
