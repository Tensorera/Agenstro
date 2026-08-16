export type TaskState =
  | "idle"
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "disabled";

export type RunState =
  | "pending"
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "timed_out"
  | "interrupted"
  | "skipped"
  | "cancelled";
export type RunTrigger = "schedule" | "manual" | "recovery";
export type PhaseName = "preprocess" | "main" | "postprocess";

export interface TaskScripts {
  preprocess: string | null;
  main: string;
  postprocess: string | null;
  helpers: string[];
}

export interface TaskSummary {
  id: string;
  name: string;
  description: string;
  cron: string;
  timezone: string;
  enabled: boolean;
  status: TaskState;
  lastRunAt: string | null;
  nextRunAt: string | null;
  targetDirectory: string;
  taskDirectory: string;
  scripts: TaskScripts;
}

export interface TaskRunSummary {
  id: string;
  taskId: string;
  status: RunState;
  trigger: RunTrigger;
  startedAt: string;
  finishedAt: string | null;
  durationMs: number | null;
  summary: string;
}

export interface RunPhase {
  name: PhaseName;
  status: RunState;
  startedAt: string | null;
  finishedAt: string | null;
  exitCode: number | null;
}

export interface RunLogEntry {
  timestamp: string;
  phase: PhaseName | "system";
  level: "debug" | "info" | "warning" | "error";
  message: string;
}

export interface RunArtifact {
  name: string;
  path: string;
  size: number | null;
}

export interface TaskRunDetail extends TaskRunSummary {
  phases: RunPhase[];
  logs: RunLogEntry[];
  artifacts: RunArtifact[];
  error: string | null;
}

export interface TaskImportResult {
  task: TaskSummary;
  warnings: string[];
}

export interface SystemStatus {
  version: string;
  schedulerRunning: boolean;
  taskCount: number;
  enabledCount: number;
  runningCount: number;
  installationRoot: string;
  startedAt: string;
  canHide: boolean;
}

export interface HideWindowResult {
  hidden: boolean;
}

export interface BridgeError {
  type: string;
  message: string;
  details?: string[];
}

export type BridgeResult<T> =
  | { ok: true; data: T; error: null }
  | { ok: false; data: null; error: BridgeError };

declare global {
  interface Window {
    pywebview?: {
      api?: Record<string, (...args: unknown[]) => unknown>;
    };
  }
}
