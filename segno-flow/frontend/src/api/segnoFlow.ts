import type {
  BridgeError,
  BridgeResult,
  HideWindowResult,
  PhaseName,
  RunArtifact,
  RunLogEntry,
  RunPhase,
  RunState,
  RunTrigger,
  SystemStatus,
  TaskImportResult,
  TaskRunDetail,
  TaskRunSummary,
  TaskScripts,
  TaskState,
  TaskSummary,
} from "../types/segnoFlow";

type BridgeMethod =
  | "system_status"
  | "task_list"
  | "task_import"
  | "task_run_now"
  | "task_set_enabled"
  | "task_runs"
  | "task_run_detail"
  | "hide_window";

type UnknownRecord = Record<string, unknown>;

export class SegnoFlowApiError extends Error {
  readonly errorType: string;
  readonly details: string[];

  constructor(error: BridgeError) {
    super(error.message);
    this.name = "SegnoFlowApiError";
    this.errorType = error.type;
    this.details = error.details ?? [];
  }
}

function isRecord(value: unknown): value is UnknownRecord {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function pick(record: UnknownRecord, camel: string, snake: string): unknown {
  return record[camel] ?? record[snake];
}

function textValue(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function nullableText(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function numberValue(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function nullableNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function booleanValue(value: unknown, fallback = false): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function enumValue<T extends string>(
  value: unknown,
  values: readonly T[],
  fallback: T,
): T {
  return typeof value === "string" && values.includes(value as T)
    ? (value as T)
    : fallback;
}

const taskStates = [
  "idle",
  "queued",
  "running",
  "succeeded",
  "failed",
  "disabled",
] as const satisfies readonly TaskState[];
const runStates = [
  "pending",
  "queued",
  "running",
  "succeeded",
  "failed",
  "timed_out",
  "interrupted",
  "skipped",
  "cancelled",
] as const satisfies readonly RunState[];
const triggers = ["schedule", "manual", "recovery"] as const satisfies readonly RunTrigger[];
const phases = ["preprocess", "main", "postprocess"] as const satisfies readonly PhaseName[];

function normalizeScripts(value: unknown): TaskScripts {
  const record = isRecord(value) ? value : {};
  const helpers = Array.isArray(record.helpers)
    ? record.helpers.filter((entry): entry is string => typeof entry === "string")
    : [];
  return {
    preprocess: nullableText(record.preprocess) ?? nullableText(record.pre),
    main: textValue(record.main, "main.py"),
    postprocess: nullableText(record.postprocess) ?? nullableText(record.post),
    helpers,
  };
}

function normalizeTask(value: unknown): TaskSummary {
  if (!isRecord(value)) {
    throw new SegnoFlowApiError({
      type: "BridgeProtocolError",
      message: "Segno Flow returned an invalid task record.",
    });
  }
  const lastRun = isRecord(pick(value, "lastRun", "last_run"))
    ? (pick(value, "lastRun", "last_run") as UnknownRecord)
    : null;
  const enabled = booleanValue(value.enabled);
  const running = booleanValue(value.running);
  const reportedStatus = value.status ?? lastRun?.status;
  const normalizedReportedStatus = reportedStatus === "timed_out" || reportedStatus === "interrupted"
    ? "failed"
    : reportedStatus === "pending"
      ? "queued"
      : reportedStatus === "skipped"
        ? "idle"
        : reportedStatus;
  const status = !enabled
    ? "disabled"
    : running
      ? "running"
      : enumValue(normalizedReportedStatus, taskStates, "idle");
  return {
    id: textValue(value.id),
    name: textValue(value.name, "Untitled task"),
    description: textValue(value.description),
    cron: textValue(value.cron, "0 * * * *"),
    timezone: textValue(value.timezone, "Local"),
    enabled,
    status,
    lastRunAt:
      nullableText(pick(value, "lastRunAt", "last_run_at")) ??
      (lastRun
        ? nullableText(pick(lastRun, "finishedAt", "finished_at")) ??
          nullableText(pick(lastRun, "startedAt", "started_at"))
        : null),
    nextRunAt: nullableText(pick(value, "nextRunAt", "next_run_at")),
    targetDirectory: textValue(
      pick(value, "targetDirectory", "target_directory") ?? value.working_directory,
    ),
    taskDirectory: textValue(pick(value, "taskDirectory", "task_directory")),
    scripts: normalizeScripts(value.scripts),
  };
}

function normalizeRun(value: unknown): TaskRunSummary {
  if (!isRecord(value)) {
    throw new SegnoFlowApiError({
      type: "BridgeProtocolError",
      message: "Segno Flow returned an invalid run record.",
    });
  }
  const durationMs = nullableNumber(pick(value, "durationMs", "duration_ms"));
  const durationSeconds = nullableNumber(value.duration_seconds);
  const status = enumValue(value.status, runStates, "queued");
  return {
    id: textValue(value.id ?? value.run_id),
    taskId: textValue(pick(value, "taskId", "task_id")),
    status,
    trigger: enumValue(value.trigger, triggers, "schedule"),
    startedAt: textValue(
      pick(value, "startedAt", "started_at") ?? pick(value, "createdAt", "created_at"),
    ),
    finishedAt: nullableText(pick(value, "finishedAt", "finished_at")),
    durationMs: durationMs ?? (durationSeconds === null ? null : durationSeconds * 1_000),
    summary: textValue(value.summary, textValue(value.error, `Run ${status}`)),
  };
}

function normalizePhase(value: unknown): RunPhase {
  const record = isRecord(value) ? value : {};
  const rawName = record.name === "pre"
    ? "preprocess"
    : record.name === "post"
      ? "postprocess"
      : record.name;
  return {
    name: enumValue(rawName, phases, "main"),
    status: enumValue(record.status, runStates, "queued"),
    startedAt: nullableText(pick(record, "startedAt", "started_at")),
    finishedAt: nullableText(pick(record, "finishedAt", "finished_at")),
    exitCode: nullableNumber(pick(record, "exitCode", "exit_code")),
  };
}

function normalizeLog(value: unknown): RunLogEntry {
  const record = isRecord(value) ? value : {};
  const logPhases = ["preprocess", "main", "postprocess", "system"] as const;
  const levels = ["debug", "info", "warning", "error"] as const;
  return {
    timestamp: textValue(record.timestamp),
    phase: enumValue(record.phase, logPhases, "system"),
    level: enumValue(record.level, levels, "info"),
    message: textValue(record.message),
  };
}

function normalizeArtifact(value: unknown): RunArtifact {
  const record = isRecord(value) ? value : {};
  return {
    name: textValue(record.name, "artifact"),
    path: textValue(record.path),
    size: nullableNumber(record.size),
  };
}

function normalizeRunDetail(value: unknown): TaskRunDetail {
  const envelope = isRecord(value) ? value : {};
  const rawRun = isRecord(envelope.run) ? envelope.run : value;
  const summary = normalizeRun(rawRun);
  const record = rawRun as UnknownRecord;
  const rawLog = textValue(envelope.log);
  const logsFromText: RunLogEntry[] = rawLog
    ? rawLog.split(/\r?\n/).filter(Boolean).map((message) => ({
        timestamp: summary.startedAt,
        phase: "system",
        level: "info",
        message,
      }))
    : [];
  return {
    ...summary,
    phases: Array.isArray(record.phases) ? record.phases.map(normalizePhase) : [],
    logs: Array.isArray(record.logs) ? record.logs.map(normalizeLog) : logsFromText,
    artifacts: Array.isArray(record.artifacts)
      ? record.artifacts.map(normalizeArtifact)
      : [],
    error: nullableText(record.error),
  };
}

function normalizeStatus(value: unknown): SystemStatus {
  if (!isRecord(value)) {
    throw new SegnoFlowApiError({
      type: "BridgeProtocolError",
      message: "Segno Flow returned an invalid system status.",
    });
  }
  const service = isRecord(value.service) ? value.service : {};
  const scheduler = isRecord(value.scheduler) ? value.scheduler : {};
  return {
    version: textValue(value.version, "unknown"),
    schedulerRunning: booleanValue(
      pick(value, "schedulerRunning", "scheduler_running") ?? scheduler.running,
    ),
    taskCount: numberValue(pick(value, "taskCount", "task_count")),
    enabledCount: numberValue(pick(value, "enabledCount", "enabled_count")),
    runningCount: numberValue(pick(value, "runningCount", "running_count")),
    installationRoot: textValue(
      pick(value, "installationRoot", "installation_root") ?? value.root,
    ),
    startedAt: textValue(
      pick(value, "startedAt", "started_at") ?? service.started_at,
    ),
    canHide: booleanValue(pick(value, "canHide", "can_hide"), true),
  };
}

function isBridgeResult<T>(value: unknown): value is BridgeResult<T> {
  if (!isRecord(value)) return false;
  return typeof value.ok === "boolean" && "data" in value && "error" in value;
}

function ok<T>(data: T): BridgeResult<T> {
  return { ok: true, data, error: null };
}

function failed(type: string, message: string, details: string[] = []): BridgeResult<never> {
  return { ok: false, data: null, error: { type, message, details } };
}

const MOCK_ROOT = "C:\\Users\\demo\\AgentroTasks";
const MOCK_TARGET = "D:\\Workspaces\\ResearchOps";

function isoOffset(minutes: number): string {
  return new Date(Date.now() + minutes * 60_000).toISOString();
}

function makeTask(overrides: Partial<TaskSummary> & Pick<TaskSummary, "id" | "name">): TaskSummary {
  return {
    id: overrides.id,
    name: overrides.name,
    description: overrides.description ?? "",
    cron: overrides.cron ?? "0 8 * * 1-5",
    timezone: overrides.timezone ?? "America/Denver",
    enabled: overrides.enabled ?? true,
    status: overrides.status ?? "idle",
    lastRunAt: overrides.lastRunAt ?? isoOffset(-120),
    nextRunAt: overrides.nextRunAt ?? isoOffset(16 * 60),
    targetDirectory: overrides.targetDirectory ?? MOCK_TARGET,
    taskDirectory:
      overrides.taskDirectory ?? `${MOCK_ROOT}\\${overrides.id}`,
    scripts: overrides.scripts ?? {
      preprocess: "preprocess.py",
      main: "main.py",
      postprocess: "postprocess.py",
      helpers: ["helpers/reporting.py", "helpers/sources.py"],
    },
  };
}

function makeRun(
  taskId: string,
  suffix: string,
  status: RunState,
  minutesAgo: number,
  trigger: RunTrigger = "schedule",
): TaskRunDetail {
  const startedAt = isoOffset(-minutesAgo);
  const finishedAt = status === "running" || status === "queued" ? null : isoOffset(-minutesAgo + 3);
  const success = status === "succeeded";
  return {
    id: `${taskId}-${suffix}`,
    taskId,
    status,
    trigger,
    startedAt,
    finishedAt,
    durationMs: finishedAt ? 184_240 : null,
    summary:
      status === "failed"
        ? "Post-process validation failed"
        : status === "running"
          ? "Workflow is collecting source material"
          : "Workflow completed and published 3 artifacts",
    phases: [
      {
        name: "preprocess",
        status: status === "running" ? "running" : "succeeded",
        startedAt,
        finishedAt: status === "running" ? null : isoOffset(-minutesAgo + 1),
        exitCode: status === "running" ? null : 0,
      },
      {
        name: "main",
        status: status === "running" ? "queued" : "succeeded",
        startedAt: status === "running" ? null : isoOffset(-minutesAgo + 1),
        finishedAt: status === "running" ? null : isoOffset(-minutesAgo + 2),
        exitCode: status === "running" ? null : 0,
      },
      {
        name: "postprocess",
        status: status === "failed" ? "failed" : status === "running" ? "queued" : "succeeded",
        startedAt: status === "running" ? null : isoOffset(-minutesAgo + 2),
        finishedAt,
        exitCode: status === "failed" ? 1 : success ? 0 : null,
      },
    ],
    logs: [
      {
        timestamp: startedAt,
        phase: "system",
        level: "info",
        message: `Run accepted from ${trigger} trigger`,
      },
      {
        timestamp: isoOffset(-minutesAgo + 1),
        phase: "preprocess",
        level: "info",
        message: "Created isolated work directory and collected 14 inputs",
      },
      ...(status === "running"
        ? []
        : [
            {
              timestamp: isoOffset(-minutesAgo + 2),
              phase: "main" as const,
              level: "info" as const,
              message: "Main workflow completed successfully",
            },
            {
              timestamp: finishedAt ?? isoOffset(-minutesAgo + 3),
              phase: "postprocess" as const,
              level: status === "failed" ? ("error" as const) : ("info" as const),
              message:
                status === "failed"
                  ? "Artifact manifest is missing report.json"
                  : "Copied artifacts to the durable output directory",
            },
          ]),
    ],
    artifacts: success
      ? [
          { name: "digest.md", path: "artifacts/digest.md", size: 18_430 },
          { name: "sources.json", path: "artifacts/sources.json", size: 6_104 },
          { name: "run.log", path: "logs/run.log", size: 24_089 },
        ]
      : [],
    error: status === "failed" ? "Required artifact report.json was not produced." : null,
  };
}

let mockTasks: TaskSummary[] = [];
let mockRuns = new Map<string, TaskRunDetail[]>();
let mockHidden = false;
let bridgeReadyPromise: Promise<Record<string, (...args: unknown[]) => unknown>> | null = null;

export function resetMockSegnoFlow(): void {
  const research = makeTask({
    id: "research-digest",
    name: "Research digest",
    description: "Collect new papers, prepare a concise digest, and archive source metadata.",
    cron: "15 7 * * 1-5",
    status: "succeeded",
    lastRunAt: isoOffset(-65),
    nextRunAt: isoOffset(23 * 60),
  });
  const feedback = makeTask({
    id: "feedback-sweep",
    name: "Feedback sweep",
    description: "Consolidate product feedback and publish the weekly triage workbook.",
    cron: "0 */6 * * *",
    status: "running",
    lastRunAt: isoOffset(-8),
    nextRunAt: isoOffset(6 * 60),
    targetDirectory: "D:\\Workspaces\\ProductSignals",
  });
  const evidence = makeTask({
    id: "release-evidence",
    name: "Release evidence pack",
    description: "Build release evidence and retain signed validation logs.",
    cron: "0 21 * * 5",
    enabled: false,
    status: "disabled",
    lastRunAt: isoOffset(-5 * 24 * 60),
    nextRunAt: null,
    targetDirectory: "D:\\Releases\\Evidence",
  });
  mockTasks = [research, feedback, evidence];
  mockRuns = new Map([
    [
      research.id,
      [
        makeRun(research.id, "104", "succeeded", 65),
        makeRun(research.id, "103", "succeeded", 24 * 60 + 65),
        makeRun(research.id, "102", "failed", 2 * 24 * 60 + 65),
      ],
    ],
    [feedback.id, [makeRun(feedback.id, "218", "running", 8)]],
    [evidence.id, [makeRun(evidence.id, "041", "failed", 5 * 24 * 60)]],
  ]);
  mockHidden = false;
  bridgeReadyPromise = null;
}

resetMockSegnoFlow();

function mockSystemStatus(): SystemStatus {
  return {
    version: "0.1.0-dev",
    schedulerRunning: true,
    taskCount: mockTasks.length,
    enabledCount: mockTasks.filter((task) => task.enabled).length,
    runningCount: mockTasks.filter((task) => task.status === "running").length,
    installationRoot: MOCK_ROOT,
    startedAt: isoOffset(-7 * 60),
    canHide: true,
  };
}

function findMockTask(taskId: unknown): TaskSummary | undefined {
  return mockTasks.find((task) => task.id === String(taskId));
}

const mockBridge: Record<BridgeMethod, (...args: unknown[]) => BridgeResult<unknown>> = {
  system_status: () => ok(mockSystemStatus()),
  task_list: () => ok({ tasks: mockTasks.map((task) => ({ ...task })) }),
  task_import: (fileName: unknown, contentBase64: unknown) => {
    const name = String(fileName);
    if (!name.toLowerCase().endsWith(".zip")) {
      return failed("ImportValidationError", "Choose a .zip task package.");
    }
    if (!String(contentBase64)) {
      return failed("ImportValidationError", "The selected archive is empty.");
    }
    if (name.toLowerCase().includes("invalid")) {
      return failed(
        "CompilationError",
        "Task package did not pass compilation checks.",
        ["manifest.json: cron is required", "scripts/main.py: invalid syntax on line 4"],
      );
    }
    const stem = name.replace(/\.zip$/i, "").replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "");
    const id = `${stem || "imported-task"}-${mockTasks.length + 1}`.toLowerCase();
    const task = makeTask({
      id,
      name: stem ? stem.replaceAll("-", " ").replace(/\b\w/g, (letter) => letter.toUpperCase()) : "Imported task",
      description: "Imported package; manifest and Python scripts passed compilation checks.",
      status: "idle",
      lastRunAt: null,
      nextRunAt: isoOffset(60),
    });
    mockTasks = [task, ...mockTasks];
    mockRuns.set(task.id, []);
    return ok<TaskImportResult>({
      task,
      warnings: ["The package uses the system default Python interpreter."],
    });
  },
  task_run_now: (taskId: unknown) => {
    const task = findMockTask(taskId);
    if (!task) return failed("TaskNotFoundError", `Unknown task: ${String(taskId)}`);
    const run = makeRun(task.id, `manual-${Date.now()}`, "succeeded", 3, "manual");
    mockRuns.set(task.id, [run, ...(mockRuns.get(task.id) ?? [])]);
    task.status = task.enabled ? "succeeded" : "disabled";
    task.lastRunAt = run.finishedAt;
    return ok(run);
  },
  task_set_enabled: (taskId: unknown, enabled: unknown) => {
    const task = findMockTask(taskId);
    if (!task) return failed("TaskNotFoundError", `Unknown task: ${String(taskId)}`);
    task.enabled = Boolean(enabled);
    task.status = task.enabled ? "idle" : "disabled";
    task.nextRunAt = task.enabled ? isoOffset(60) : null;
    return ok({ ...task });
  },
  task_runs: (taskId: unknown) => {
    if (!findMockTask(taskId)) {
      return failed("TaskNotFoundError", `Unknown task: ${String(taskId)}`);
    }
    return ok({ runs: (mockRuns.get(String(taskId)) ?? []).map((run) => ({ ...run })) });
  },
  task_run_detail: (taskId: unknown, runId: unknown) => {
    const run = (mockRuns.get(String(taskId)) ?? []).find(
      (candidate) => candidate.id === String(runId),
    );
    return run
      ? ok({ ...run })
      : failed("RunNotFoundError", `Unknown run: ${String(runId)}`);
  },
  hide_window: () => {
    mockHidden = true;
    return ok<HideWindowResult>({ hidden: mockHidden });
  },
};

function waitForProductionBridge(
  method: BridgeMethod,
): Promise<Record<string, (...args: unknown[]) => unknown>> {
  if (typeof window.pywebview?.api?.[method] === "function") {
    return Promise.resolve(window.pywebview.api);
  }
  if (!bridgeReadyPromise) {
    bridgeReadyPromise = new Promise((resolve, reject) => {
      let timeout = 0;
      const ready = () => {
        window.clearTimeout(timeout);
        const api = window.pywebview?.api;
        if (api) {
          resolve(api);
        } else {
          reject(
            new SegnoFlowApiError({
              type: "BridgeUnavailableError",
              message: "Segno Flow bridge announced readiness without an API.",
            }),
          );
        }
      };
      timeout = window.setTimeout(() => {
        window.removeEventListener("pywebviewready", ready);
        reject(
          new SegnoFlowApiError({
            type: "BridgeUnavailableError",
            message: "Segno Flow service did not become ready.",
          }),
        );
      }, 5_000);
      window.addEventListener("pywebviewready", ready, { once: true });
    });
  }
  return bridgeReadyPromise.then((api) => {
    if (typeof api[method] !== "function") {
      throw new SegnoFlowApiError({
        type: "BridgeProtocolError",
        message: `Segno Flow bridge does not expose ${method}.`,
      });
    }
    return api;
  });
}

async function invoke<T>(method: BridgeMethod, ...args: unknown[]): Promise<T> {
  const useMock = import.meta.env.DEV || import.meta.env.MODE === "test";
  const api = useMock ? window.pywebview?.api : await waitForProductionBridge(method);
  const bridgeMethod = api?.[method];
  const raw =
    typeof bridgeMethod === "function"
      ? await bridgeMethod.apply(api, args)
      : useMock
        ? await Promise.resolve(mockBridge[method](...args))
        : undefined;
  if (!isBridgeResult<T>(raw)) {
    throw new SegnoFlowApiError({
      type: "BridgeProtocolError",
      message: `${method} returned an invalid bridge envelope.`,
    });
  }
  if (!raw.ok) throw new SegnoFlowApiError(raw.error);
  return raw.data;
}

function collection(value: unknown, key: "tasks" | "runs"): unknown[] {
  if (Array.isArray(value)) return value;
  if (isRecord(value) && Array.isArray(value[key])) return value[key];
  throw new SegnoFlowApiError({
    type: "BridgeProtocolError",
    message: `Segno Flow returned an invalid ${key} collection.`,
  });
}

export const segnoFlowApi = {
  systemStatus: (): Promise<SystemStatus> =>
    invoke("system_status").then(normalizeStatus),
  taskList: (): Promise<TaskSummary[]> =>
    invoke("task_list").then((value) => collection(value, "tasks").map(normalizeTask)),
  taskImport: (
    fileName: string,
    contentBase64: string,
  ): Promise<TaskImportResult> =>
    invoke<unknown>("task_import", fileName, contentBase64).then((value) => {
      if (!isRecord(value)) {
        throw new SegnoFlowApiError({
          type: "BridgeProtocolError",
          message: "Segno Flow returned an invalid import result.",
        });
      }
      return {
        task: normalizeTask(value.task),
        warnings: Array.isArray(value.warnings)
          ? value.warnings.filter((warning): warning is string => typeof warning === "string")
          : [],
      };
    }),
  taskRunNow: (taskId: string): Promise<TaskRunSummary> =>
    invoke<unknown>("task_run_now", taskId).then((value) => {
      if (isRecord(value) && typeof value.run_id === "string" && !("status" in value)) {
        return normalizeRun({
          run_id: value.run_id,
          task_id: value.task_id ?? taskId,
          status: "queued",
          trigger: "manual",
          started_at: new Date().toISOString(),
          summary: value.accepted === false ? "Run was not accepted" : "Run accepted",
        });
      }
      return normalizeRun(value);
    }),
  taskSetEnabled: (taskId: string, enabled: boolean): Promise<TaskSummary> =>
    invoke<unknown>("task_set_enabled", taskId, enabled).then((value) =>
      normalizeTask(isRecord(value) && "task" in value ? value.task : value),
    ),
  taskRuns: (taskId: string): Promise<TaskRunSummary[]> =>
    invoke("task_runs", taskId).then((value) =>
      collection(value, "runs").map(normalizeRun),
    ),
  taskRunDetail: (taskId: string, runId: string): Promise<TaskRunDetail> =>
    invoke("task_run_detail", taskId, runId).then(normalizeRunDetail),
  hideWindow: (): Promise<HideWindowResult> => invoke("hide_window"),
};

export function usingMockBridge(): boolean {
  return (
    (import.meta.env.DEV || import.meta.env.MODE === "test") &&
    typeof window.pywebview?.api === "undefined"
  );
}
