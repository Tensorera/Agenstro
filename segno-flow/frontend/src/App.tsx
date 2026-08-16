import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ImportDialog } from "./components/ImportDialog";
import { Icon } from "./components/Icon";
import { TaskSidebar, type TaskFilter } from "./components/TaskSidebar";
import { TaskWorkspace } from "./components/TaskWorkspace";
import {
  SegnoFlowApiError,
  segnoFlowApi,
  usingMockBridge,
} from "./api/segnoFlow";
import type {
  SystemStatus,
  TaskRunDetail,
  TaskRunSummary,
  TaskSummary,
} from "./types/segnoFlow";

const MAX_ARCHIVE_BYTES = 64 * 1_048_576;
const RUN_REFRESH_INTERVAL_MS = 2_500;

function runIsActive(status: TaskRunSummary["status"] | undefined): boolean {
  return status === "pending" || status === "queued" || status === "running";
}

interface UiError {
  title: string;
  message: string;
  details: string[];
}

function errorFrom(caught: unknown, title: string): UiError {
  if (caught instanceof SegnoFlowApiError) {
    return { title, message: caught.message, details: caught.details };
  }
  return {
    title,
    message: caught instanceof Error ? caught.message : String(caught),
    details: [],
  };
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("The selected archive could not be read."));
    reader.onload = () => {
      const result = reader.result;
      if (typeof result !== "string") {
        reject(new Error("The selected archive could not be encoded."));
        return;
      }
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.readAsDataURL(file);
  });
}

function EmptyWorkspace({ onImport }: { onImport: () => void }) {
  return (
    <section className="empty-workspace">
      <div className="empty-illustration" aria-hidden="true">
        <span><Icon name="calendar" /></span>
        <span><Icon name="play" /></span>
        <span><Icon name="archive" /></span>
      </div>
      <p className="eyebrow">No workflows registered</p>
      <h1>Put recurring work on a durable schedule.</h1>
      <p>
        Import a compiled task package to run its preparation, main workflow, and
        artifact collection steps on a cron schedule.
      </p>
      <button type="button" className="primary-button" onClick={onImport}>
        <Icon name="import" /> Import your first task
      </button>
      <span className="empty-hint">The original ZIP remains unchanged during validation.</span>
    </section>
  );
}

export default function App() {
  const [system, setSystem] = useState<SystemStatus | null>(null);
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [runs, setRuns] = useState<TaskRunSummary[]>([]);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [runDetail, setRunDetail] = useState<TaskRunDetail | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<TaskFilter>("all");
  const [initialLoading, setInitialLoading] = useState(true);
  const [runsLoading, setRunsLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [togglingTaskId, setTogglingTaskId] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [importBusy, setImportBusy] = useState(false);
  const [hideBusy, setHideBusy] = useState(false);
  const [error, setError] = useState<UiError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const loadGeneration = useRef(0);
  const selectedTaskIdRef = useRef<string | null>(null);
  const selectedRunIdRef = useRef<string | null>(null);
  const runsRef = useRef<TaskRunSummary[]>([]);
  const runDetailRef = useRef<TaskRunDetail | null>(null);
  selectedTaskIdRef.current = selectedTaskId;
  selectedRunIdRef.current = selectedRunId;
  runsRef.current = runs;
  runDetailRef.current = runDetail;

  const selectRun = useCallback((runId: string | null) => {
    selectedRunIdRef.current = runId;
    setSelectedRunId(runId);
  }, []);

  const applyRuns = useCallback((nextRuns: TaskRunSummary[]) => {
    runsRef.current = nextRuns;
    setRuns(nextRuns);
  }, []);

  const selectedTask = useMemo(
    () => tasks.find((task) => task.id === selectedTaskId) ?? null,
    [selectedTaskId, tasks],
  );

  const visibleTasks = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return tasks.filter((task) => {
      const matchesQuery =
        !normalized ||
        `${task.name} ${task.description} ${task.cron}`.toLocaleLowerCase().includes(normalized);
      const matchesFilter =
        filter === "all" ||
        (filter === "disabled" ? !task.enabled : task.status === filter);
      return matchesQuery && matchesFilter;
    });
  }, [filter, query, tasks]);

  const loadOverview = useCallback(async () => {
    setInitialLoading(true);
    setError(null);
    try {
      const [nextSystem, nextTasks] = await Promise.all([
        segnoFlowApi.systemStatus(),
        segnoFlowApi.taskList(),
      ]);
      setSystem(nextSystem);
      setTasks(nextTasks);
      setSelectedTaskId((current) =>
        current && nextTasks.some((task) => task.id === current)
          ? current
          : (nextTasks[0]?.id ?? null),
      );
    } catch (caught) {
      setError(errorFrom(caught, "Segno Flow could not be opened"));
    } finally {
      setInitialLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadOverview();
  }, [loadOverview]);

  useEffect(() => {
    const identifier = window.setInterval(() => {
      void Promise.all([segnoFlowApi.systemStatus(), segnoFlowApi.taskList()])
        .then(([nextSystem, nextTasks]) => {
          setSystem(nextSystem);
          setTasks(nextTasks);
        })
        .catch(() => {
          // Keep the last durable snapshot when a background refresh fails.
        });
    }, 5_000);
    return () => window.clearInterval(identifier);
  }, []);

  useEffect(() => {
    if (!selectedTaskId) {
      applyRuns([]);
      selectRun(null);
      runDetailRef.current = null;
      setRunDetail(null);
      return;
    }
    const generation = ++loadGeneration.current;
    setRunsLoading(true);
    selectRun(null);
    runDetailRef.current = null;
    setRunDetail(null);
    void segnoFlowApi.taskRuns(selectedTaskId)
      .then((nextRuns) => {
        if (generation !== loadGeneration.current) return;
        applyRuns(nextRuns);
        selectRun(nextRuns[0]?.id ?? null);
      })
      .catch((caught) => {
        if (generation === loadGeneration.current) {
          setError(errorFrom(caught, "Run history could not be loaded"));
          applyRuns([]);
        }
      })
      .finally(() => {
        if (generation === loadGeneration.current) setRunsLoading(false);
      });
  }, [applyRuns, selectRun, selectedTaskId]);

  useEffect(() => {
    if (!selectedTaskId || !selectedRunId) {
      runDetailRef.current = null;
      setRunDetail(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    void segnoFlowApi.taskRunDetail(selectedTaskId, selectedRunId)
      .then((detail) => {
        if (!cancelled) {
          runDetailRef.current = detail;
          setRunDetail(detail);
        }
      })
      .catch((caught) => {
        if (cancelled) return;
        const queued = runsRef.current.find((run) => run.id === selectedRunId);
        if (queued?.status === "queued") {
          const queuedDetail: TaskRunDetail = {
            ...queued,
            phases: [],
            logs: [{
              timestamp: queued.startedAt,
              phase: "system",
              level: "info",
              message: "Run accepted and waiting for a worker.",
            }],
            artifacts: [],
            error: null,
          };
          runDetailRef.current = queuedDetail;
          setRunDetail(queuedDetail);
        } else {
          setError(errorFrom(caught, "Run details could not be loaded"));
        }
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedRunId, selectedTaskId]);

  useEffect(() => {
    if (!selectedTaskId) return;
    const taskId = selectedTaskId;
    let cancelled = false;
    let refreshing = false;

    const refreshSelectedRuns = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        const nextRuns = await segnoFlowApi.taskRuns(taskId);
        if (cancelled || selectedTaskIdRef.current !== taskId) return;
        applyRuns(nextRuns);

        const currentRunId = selectedRunIdRef.current;
        const currentStillExists = Boolean(
          currentRunId && nextRuns.some((run) => run.id === currentRunId),
        );
        const nextRunId = currentStillExists ? currentRunId : (nextRuns[0]?.id ?? null);
        if (nextRunId !== currentRunId) {
          selectRun(nextRunId);
          return;
        }
        if (!nextRunId) return;

        const summary = nextRuns.find((run) => run.id === nextRunId);
        const selectedDetail = runDetailRef.current;
        const detailNeedsRefresh =
          runIsActive(summary?.status) ||
          (selectedDetail?.id === nextRunId && runIsActive(selectedDetail.status));
        if (!detailNeedsRefresh) return;

        const nextDetail = await segnoFlowApi.taskRunDetail(taskId, nextRunId);
        if (
          cancelled ||
          selectedTaskIdRef.current !== taskId ||
          selectedRunIdRef.current !== nextRunId
        ) {
          return;
        }
        runDetailRef.current = nextDetail;
        setRunDetail(nextDetail);
      } catch {
        // Background polling keeps the last durable history and detail snapshot.
      } finally {
        refreshing = false;
      }
    };

    const identifier = window.setInterval(
      () => void refreshSelectedRuns(),
      RUN_REFRESH_INTERVAL_MS,
    );
    return () => {
      cancelled = true;
      window.clearInterval(identifier);
    };
  }, [applyRuns, selectRun, selectedTaskId]);

  useEffect(() => {
    if (!notice) return;
    const identifier = window.setTimeout(() => setNotice(null), 4_500);
    return () => window.clearTimeout(identifier);
  }, [notice]);

  const refreshSystem = async () => {
    try {
      setSystem(await segnoFlowApi.systemStatus());
    } catch {
      // Action success is retained even if the aggregate count refresh fails.
    }
  };

  const runNow = async () => {
    if (!selectedTask || runningTaskId) return;
    setRunningTaskId(selectedTask.id);
    setError(null);
    try {
      const accepted = await segnoFlowApi.taskRunNow(selectedTask.id);
      const [nextTasks, nextRuns] = await Promise.all([
        segnoFlowApi.taskList(),
        segnoFlowApi.taskRuns(selectedTask.id),
      ]);
      const hasAccepted = nextRuns.some((run) => run.id === accepted.id);
      setTasks(nextTasks);
      applyRuns(hasAccepted ? nextRuns : [accepted, ...nextRuns]);
      selectRun(accepted.id);
      setNotice(`Run ${accepted.id} was accepted.`);
      void refreshSystem();
    } catch (caught) {
      setError(errorFrom(caught, "The task could not be started"));
    } finally {
      setRunningTaskId(null);
    }
  };

  const toggleTask = async () => {
    if (!selectedTask || togglingTaskId) return;
    setTogglingTaskId(selectedTask.id);
    setError(null);
    try {
      const updated = await segnoFlowApi.taskSetEnabled(
        selectedTask.id,
        !selectedTask.enabled,
      );
      setTasks((current) =>
        current.map((task) => task.id === updated.id ? updated : task),
      );
      setNotice(`${updated.name} is now ${updated.enabled ? "enabled" : "paused"}.`);
      void refreshSystem();
    } catch (caught) {
      setError(errorFrom(caught, "The schedule could not be updated"));
    } finally {
      setTogglingTaskId(null);
    }
  };

  const importTask = async (file: File) => {
    setError(null);
    if (!file.name.toLowerCase().endsWith(".zip")) {
      setError({
        title: "This package cannot be imported",
        message: "Choose a ZIP archive containing segno-flow.json and the workflow scripts.",
        details: [],
      });
      return;
    }
    if (file.size > MAX_ARCHIVE_BYTES) {
      setError({
        title: "This package is too large",
        message: "Task archives must be 64 MB or smaller.",
        details: [],
      });
      return;
    }
    setImportBusy(true);
    try {
      const contentBase64 = await fileToBase64(file);
      const result = await segnoFlowApi.taskImport(file.name, contentBase64);
      setTasks((current) => [
        result.task,
        ...current.filter((task) => task.id !== result.task.id),
      ]);
      setSelectedTaskId(result.task.id);
      setQuery("");
      setFilter("all");
      setImportOpen(false);
      setNotice(
        result.warnings.length
          ? `${result.task.name} imported with ${result.warnings.length} warning${result.warnings.length === 1 ? "" : "s"}.`
          : `${result.task.name} passed compilation and was imported.`,
      );
      void refreshSystem();
    } catch (caught) {
      setError(errorFrom(caught, "This package did not pass import checks"));
    } finally {
      setImportBusy(false);
    }
  };

  const hideWindow = async () => {
    if (hideBusy) return;
    setHideBusy(true);
    setError(null);
    try {
      await segnoFlowApi.hideWindow();
      if (usingMockBridge()) {
        setNotice("Desktop mode minimizes this window while the scheduler keeps running.");
      }
    } catch (caught) {
      setError(errorFrom(caught, "The window could not be hidden"));
    } finally {
      setHideBusy(false);
    }
  };

  return (
    <main className="segno-flow-shell">
      <header className="app-header">
        <div className="flow-brand">
          <span className="flow-mark" aria-hidden="true"><i /><i /><i /></span>
          <span><strong>AGENTRO</strong><small>Segno Flow</small></span>
        </div>
        <div className="service-summary" aria-live="polite">
          <span className={`service-indicator ${system?.schedulerRunning ? "online" : "offline"}`} aria-hidden="true" />
          <span>
            <strong>{system?.schedulerRunning ? "Scheduler online" : "Scheduler unavailable"}</strong>
            <small>{system ? `${system.enabledCount} enabled · ${system.runningCount} running` : "Connecting…"}</small>
          </span>
        </div>
        <div className="install-path" title={system?.installationRoot}>
          <Icon name="folder" />
          <span><small>Task library</small><strong>{system?.installationRoot || "Loading installation directory…"}</strong></span>
        </div>
        {usingMockBridge() ? <span className="mock-badge">Browser demo</span> : null}
        <button
          type="button"
          className="header-button"
          disabled={hideBusy || system?.canHide === false}
          onClick={() => void hideWindow()}
        >
          <Icon name="tray" />
          {hideBusy ? "Minimizing…" : "Minimize to background"}
        </button>
      </header>

      {error ? (
        <div className="error-banner" role="alert">
          <Icon name="warning" />
          <div>
            <strong>{error.title}</strong>
            <span>{error.message}</span>
            {error.details.length ? <ul>{error.details.map((detail) => <li key={detail}>{detail}</li>)}</ul> : null}
          </div>
          {initialLoading ? <button type="button" onClick={() => void loadOverview()}>Retry</button> : null}
          <button type="button" className="icon-button" aria-label="Dismiss error" onClick={() => setError(null)}><Icon name="close" /></button>
        </div>
      ) : null}

      <div className="app-body">
        <TaskSidebar
          tasks={visibleTasks}
          selectedId={selectedTaskId}
          query={query}
          filter={filter}
          loading={initialLoading}
          onQueryChange={setQuery}
          onFilterChange={setFilter}
          onSelect={setSelectedTaskId}
          onImport={() => setImportOpen(true)}
        />
        {initialLoading ? (
          <section className="workspace-loading" aria-label="Loading Segno Flow">
            <div className="large-spinner" />
            <strong>Opening Segno Flow</strong>
            <span>Restoring scheduled workflows and recent runs…</span>
          </section>
        ) : selectedTask ? (
          <TaskWorkspace
            task={selectedTask}
            system={system}
            runs={runs}
            selectedRunId={selectedRunId}
            runDetail={runDetail}
            runsLoading={runsLoading}
            detailLoading={detailLoading}
            runningNow={runningTaskId === selectedTask.id}
            toggling={togglingTaskId === selectedTask.id}
            onRunNow={() => void runNow()}
            onToggle={() => void toggleTask()}
            onSelectRun={selectRun}
          />
        ) : (
          <EmptyWorkspace onImport={() => setImportOpen(true)} />
        )}
      </div>

      <footer className="status-bar">
        <span><span className={system?.schedulerRunning ? "status-light online" : "status-light"} />{system?.schedulerRunning ? "Monitoring schedules" : "Service offline"}</span>
        <span>{tasks.length} task{tasks.length === 1 ? "" : "s"}</span>
        <span className="status-spacer" />
        <span>{usingMockBridge() ? "Local mock API" : `Segno Flow ${system?.version ?? ""}`}</span>
      </footer>

      <ImportDialog
        open={importOpen}
        busy={importBusy}
        onClose={() => setImportOpen(false)}
        onImport={(file) => void importTask(file)}
      />

      {notice ? (
        <div className="toast" role="status">
          <Icon name="check" />
          <span>{notice}</span>
          <button type="button" className="icon-button" aria-label="Dismiss notification" onClick={() => setNotice(null)}><Icon name="close" /></button>
        </div>
      ) : null}
    </main>
  );
}
