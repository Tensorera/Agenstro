import type { ActionKind, StudioRun } from "../shared/contracts";
import { MAX_RENDERED_OUTPUT } from "./model";

export function appendBounded(current: string, chunk: string): string {
  const appended = current + chunk;
  if (appended.length <= MAX_RENDERED_OUTPUT) return appended;
  return `[older output omitted]\n${appended.slice(-MAX_RENDERED_OUTPUT)}`;
}

export function actionLabel(kind: ActionKind): string {
  switch (kind) {
    case "generate":
      return "Generate workflow";
    case "check":
      return "Type-check workflow";
    case "run":
      return "Run workflow";
    case "smoke":
      return "Plugin smoke test";
  }
}

export function runState(run: StudioRun): string {
  return run.outcome?.kind ?? run.state;
}

export function runTone(run: StudioRun): string {
  const state = runState(run);
  if (state === "succeeded") return "succeeded";
  if (state === "open") return "running";
  if (state === "cancelled") return "cancelled";
  return state.includes("fail") || run.integrity === "corrupt" ? "failed" : "warning";
}

export function formatEventData(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    const text = JSON.stringify(value, null, 2) ?? String(value);
    return text.length > 4_000 ? `${text.slice(0, 4_000)}\n… payload clipped in view` : text;
  } catch {
    return "[unrenderable event payload]";
  }
}

export function formatTime(unixMs: string): string {
  const date = new Date(Number(unixMs));
  if (Number.isNaN(date.valueOf())) return "unknown time";
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

export function formatDateTime(unixMs: string): string {
  const date = new Date(Number(unixMs));
  if (Number.isNaN(date.valueOf())) return "unknown";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function dayPeriod(): string {
  const hour = new Date().getHours();
  if (hour < 12) return "morning";
  if (hour < 18) return "afternoon";
  return "evening";
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = (error as { readonly message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return "The operation failed.";
}
