import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ActionRequest,
  StudioActionEvent,
  StudioEvent,
  StudioEventPage,
  StudioView,
} from "../shared/contracts";
import type { SessionAnswerInput, SessionList, SessionView } from "../shared/session-contracts";
import { ActionConsole } from "./components/ActionConsole";
import { OverviewView } from "./components/OverviewView";
import { PluginsView } from "./components/PluginsView";
import { RunsView } from "./components/RunsView";
import { SessionsView } from "./components/SessionsView";
import { TasksView } from "./components/TasksView";
import { ErrorToast, LoadingView, StudioHeader, StudioSidebar } from "./components/StudioChrome";
import { WorkflowView } from "./components/WorkflowView";
import { appendBounded, errorMessage } from "./format";
import { useTasks } from "./useTasks";
import {
  EVENT_PAGE_SIZE,
  isActionBusy,
  type ActiveAction,
  type NavigationView,
  type OutputStream,
} from "./model";

export default function App() {
  const bridgeAvailable = "motivo" in window;
  const [view, setView] = useState<NavigationView>("tasks");
  const [studio, setStudio] = useState<StudioView | null>(null);
  const [loading, setLoading] = useState(bridgeAvailable);
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [error, setError] = useState<string | null>(
    bridgeAvailable ? null : "The secure Motivo bridge is unavailable.",
  );
  const [activeAction, setActiveAction] = useState<ActiveAction | null>(null);
  const [outputStream, setOutputStream] = useState<OutputStream>("stdout");
  const [goal, setGoal] = useState("");
  const [provider, setProvider] = useState("");
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [eventPage, setEventPage] = useState<StudioEventPage | null>(null);
  const [events, setEvents] = useState<readonly StudioEvent[]>([]);
  const [eventsBusy, setEventsBusy] = useState(false);
  const [actionStartBusy, setActionStartBusy] = useState(false);
  const [sessionList, setSessionList] = useState<SessionList | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [session, setSession] = useState<SessionView | null>(null);
  const [sessionListBusy, setSessionListBusy] = useState(false);
  const [sessionCurrentBusy, setSessionCurrentBusy] = useState(false);
  const [sessionAnswerBusy, setSessionAnswerBusy] = useState(false);
  const actionIdRef = useRef<string | null>(null);
  const eventRequestRef = useRef(0);
  const workspaceHandleRef = useRef<string | null>(null);
  const selectedSessionIdRef = useRef<string | null>(null);
  const sessionListRequestRef = useRef(0);
  const sessionCurrentRequestRef = useRef(0);
  const sessionAnswerRequestRef = useRef(0);
  const sessionListOperationRef = useRef(false);
  const sessionCurrentOperationRef = useRef(false);
  const sessionAnswerOperationRef = useRef(false);
  const actionOperationRef = useRef(false);

  const acceptStudio = useCallback((next: StudioView | null) => {
    const nextHandle = next?.handle ?? null;
    if (workspaceHandleRef.current !== nextHandle) {
      workspaceHandleRef.current = nextHandle;
      sessionListRequestRef.current += 1;
      sessionCurrentRequestRef.current += 1;
      sessionAnswerRequestRef.current += 1;
      sessionListOperationRef.current = false;
      sessionCurrentOperationRef.current = false;
      sessionAnswerOperationRef.current = false;
      selectedSessionIdRef.current = null;
      setSessionList(null);
      setSelectedSessionId(null);
      setSession(null);
      setSessionListBusy(false);
      setSessionCurrentBusy(false);
      setSessionAnswerBusy(false);
    }
    setStudio(next);
    if (!next) {
      setProvider("");
      setSelectedRunId(null);
      return;
    }
    setProvider((current) =>
      next.snapshot.registries.providers.some((item) => item.name === current)
        ? current
        : next.snapshot.registries.defaultProvider,
    );
    setSelectedRunId((current) =>
      next.snapshot.runs.some((run) => run.runId === current)
        ? current
        : (next.snapshot.runs[0]?.runId ?? null),
    );
  }, []);

  const refresh = useCallback(async () => {
    const next = await window.motivo.studio.refresh();
    acceptStudio(next);
    return next;
  }, [acceptStudio]);

  useEffect(() => {
    if (!bridgeAvailable) return;

    let disposed = false;
    const unsubscribe = window.motivo.actions.subscribe((event) => {
      if (!disposed) applyActionEvent(event);
    });
    void window.motivo.studio
      .current()
      .then((current) => {
        if (!disposed) acceptStudio(current);
      })
      .catch((caught: unknown) => {
        if (!disposed) setError(errorMessage(caught));
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });

    return () => {
      disposed = true;
      unsubscribe();
    };

    function applyActionEvent(event: StudioActionEvent): void {
      if (event.type === "started") {
        actionOperationRef.current = true;
        actionIdRef.current = event.actionId;
        setActiveAction({
          actionId: event.actionId,
          kind: event.kind,
          startedAtUnixMs: event.startedAtUnixMs,
          status: "running",
          stdout: "",
          stderr: "",
          stdoutChunks: 0,
          stderrChunks: 0,
          presentations: [],
        });
        setOutputStream("stdout");
        return;
      }
      if (event.actionId !== actionIdRef.current) return;

      if (event.type === "output") {
        setActiveAction((current) => {
          if (!current || current.actionId !== event.actionId) return current;
          const counter = event.stream === "stdout" ? "stdoutChunks" : "stderrChunks";
          return {
            ...current,
            [event.stream]: appendBounded(current[event.stream], event.text),
            [counter]: current[counter] + 1,
            presentations: event.presentation
              ? [...current.presentations, { sequence: event.sequence, ...event.presentation }]
              : current.presentations,
          };
        });
        return;
      }

      actionOperationRef.current = false;
      setActionStartBusy(false);
      setActiveAction((current) =>
        current?.actionId === event.actionId
          ? {
              ...current,
              status: event.status,
              exitCode: event.exitCode,
              message: event.message,
            }
          : current,
      );
      void refresh().catch((caught: unknown) => setError(errorMessage(caught)));
    }
  }, [acceptStudio, bridgeAvailable, refresh]);

  const snapshot = studio?.snapshot;
  const pluginCount = useMemo(
    () =>
      (snapshot?.registries.providers.length ?? 0) +
      (snapshot?.registries.effects.length ?? 0) +
      (snapshot?.registries.plugins.length ?? 0),
    [snapshot],
  );

  const selectedRun = snapshot?.runs.find((run) => run.runId === selectedRunId) ?? null;
  const actionBusy = isActionBusy(activeAction) || actionStartBusy;
  const taskController = useTasks(
    studio?.handle,
    actionBusy || sessionAnswerBusy || workspaceBusy,
    setError,
  );
  const taskBusy = taskController.operating || taskController.running || taskController.loading;
  const actionControlsBusy = actionBusy || sessionAnswerBusy || taskBusy;

  useEffect(() => {
    if (view !== "runs" || !selectedRunId) return;
    void loadEvents(selectedRunId, "0", false);
  }, [selectedRunId, view]);

  useEffect(() => {
    if (view !== "sessions" || !studio || actionBusy || sessionAnswerOperationRef.current) return;
    void loadSessionList(studio.handle);
  }, [actionBusy, studio, view]);

  useEffect(() => {
    if (view !== "sessions" || !studio || !selectedSessionId) return;
    void loadSessionCurrent(selectedSessionId, studio.handle);
  }, [selectedSessionId, studio, view]);

  async function loadSessionList(workspaceHandle: string): Promise<void> {
    if (
      sessionListOperationRef.current ||
      actionOperationRef.current ||
      sessionAnswerOperationRef.current
    ) {
      return;
    }
    const request = ++sessionListRequestRef.current;
    sessionListOperationRef.current = true;
    setSessionListBusy(true);
    setError(null);
    try {
      const list = await window.motivo.sessions.list({ workspaceHandle, limit: 50 });
      if (
        request !== sessionListRequestRef.current ||
        workspaceHandleRef.current !== workspaceHandle
      ) {
        return;
      }
      setSessionList(list);
      const preferred = selectedSessionIdRef.current;
      const nextId =
        (preferred && list.sessions.some((item) => item.sessionId === preferred)
          ? preferred
          : list.sessions[0]?.sessionId) ?? null;
      selectedSessionIdRef.current = nextId;
      setSelectedSessionId(nextId);
      setSession(list.sessions.find((item) => item.sessionId === nextId) ?? null);
    } catch (caught) {
      if (
        request === sessionListRequestRef.current &&
        workspaceHandleRef.current === workspaceHandle
      ) {
        setError(errorMessage(caught));
      }
    } finally {
      if (request === sessionListRequestRef.current) {
        sessionListOperationRef.current = false;
        setSessionListBusy(false);
      }
    }
  }

  async function loadSessionCurrent(
    sessionId: string,
    workspaceHandle: string,
    allowDuringAnswer = false,
  ): Promise<void> {
    if (
      sessionCurrentOperationRef.current ||
      actionOperationRef.current ||
      (!allowDuringAnswer && sessionAnswerOperationRef.current)
    ) {
      return;
    }
    const request = ++sessionCurrentRequestRef.current;
    sessionCurrentOperationRef.current = true;
    setSessionCurrentBusy(true);
    setError(null);
    try {
      const current = await window.motivo.sessions.current({ workspaceHandle, sessionId });
      if (
        request !== sessionCurrentRequestRef.current ||
        workspaceHandleRef.current !== workspaceHandle ||
        selectedSessionIdRef.current !== sessionId
      ) {
        return;
      }
      setSession(current);
      setSessionList((list) =>
        list
          ? {
              ...list,
              sessions: list.sessions.map((item) =>
                item.sessionId === current.sessionId ? current : item,
              ),
            }
          : list,
      );
    } catch (caught) {
      if (
        request === sessionCurrentRequestRef.current &&
        workspaceHandleRef.current === workspaceHandle
      ) {
        setError(errorMessage(caught));
      }
    } finally {
      if (request === sessionCurrentRequestRef.current) {
        sessionCurrentOperationRef.current = false;
        setSessionCurrentBusy(false);
      }
    }
  }

  async function chooseWorkspace(operation: "open" | "initialize"): Promise<void> {
    if (
      !("motivo" in window) ||
      taskController.operationRef.current ||
      taskController.runningRef.current ||
      actionOperationRef.current ||
      sessionAnswerOperationRef.current ||
      sessionListOperationRef.current ||
      sessionCurrentOperationRef.current
    ) {
      return;
    }
    setWorkspaceBusy(true);
    setError(null);
    try {
      const selected =
        operation === "open"
          ? await window.motivo.studio.openInitialized()
          : await window.motivo.studio.initialize();
      if (selected) {
        acceptStudio(selected);
        setView("tasks");
        setEvents([]);
        setEventPage(null);
      }
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function refreshWorkspace(): Promise<void> {
    if (
      !studio ||
      taskController.operationRef.current ||
      taskController.runningRef.current ||
      actionOperationRef.current ||
      sessionAnswerOperationRef.current ||
      sessionListOperationRef.current ||
      sessionCurrentOperationRef.current
    ) {
      return;
    }
    setWorkspaceBusy(true);
    setError(null);
    try {
      await refresh();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setWorkspaceBusy(false);
    }
  }

  async function startAction(request: ActionRequest): Promise<void> {
    if (
      !studio ||
      taskController.operationRef.current ||
      taskController.runningRef.current ||
      taskController.loading ||
      actionBusy ||
      actionOperationRef.current ||
      sessionAnswerOperationRef.current ||
      sessionListOperationRef.current ||
      sessionCurrentOperationRef.current
    ) {
      return;
    }
    actionOperationRef.current = true;
    setActionStartBusy(true);
    setError(null);
    try {
      const started = await window.motivo.actions.start(request);
      actionIdRef.current = started.actionId;
      setActiveAction((current) =>
        current?.actionId === started.actionId
          ? current
          : {
              ...started,
              status: "running",
              stdout: "",
              stderr: "",
              stdoutChunks: 0,
              stderrChunks: 0,
              presentations: [],
            },
      );
      setOutputStream("stdout");
    } catch (caught) {
      actionOperationRef.current = false;
      setError(errorMessage(caught));
    } finally {
      setActionStartBusy(false);
    }
  }

  async function cancelAction(): Promise<void> {
    if (!activeAction || !isActionBusy(activeAction)) return;
    setActiveAction((current) => (current ? { ...current, status: "cancelling" } : current));
    try {
      await window.motivo.actions.cancel({ actionId: activeAction.actionId });
    } catch (caught) {
      setActiveAction((current) =>
        current?.actionId === activeAction.actionId ? { ...current, status: "running" } : current,
      );
      setError(errorMessage(caught));
    }
  }

  async function loadEvents(runId: string, after: string, append: boolean): Promise<void> {
    const request = ++eventRequestRef.current;
    setEventsBusy(true);
    setError(null);
    try {
      const page = await window.motivo.runs.events({ runId, after, limit: EVENT_PAGE_SIZE });
      if (request !== eventRequestRef.current || page.run.runId !== runId) return;
      setEventPage(page);
      setEvents((current) => (append ? [...current, ...page.events] : page.events));
    } catch (caught) {
      if (request === eventRequestRef.current) setError(errorMessage(caught));
    } finally {
      if (request === eventRequestRef.current) setEventsBusy(false);
    }
  }

  function selectRun(runId: string): void {
    if (runId === selectedRunId) return;
    eventRequestRef.current += 1;
    setSelectedRunId(runId);
    setEventPage(null);
    setEvents([]);
  }

  function selectSession(sessionId: string): void {
    if (sessionId === selectedSessionIdRef.current) return;
    sessionCurrentRequestRef.current += 1;
    sessionCurrentOperationRef.current = false;
    selectedSessionIdRef.current = sessionId;
    setSelectedSessionId(sessionId);
    setSession(null);
  }

  async function answerSession(input: Omit<SessionAnswerInput, "workspaceHandle">): Promise<void> {
    if (
      !studio ||
      taskController.operationRef.current ||
      taskController.runningRef.current ||
      actionBusy ||
      actionOperationRef.current ||
      sessionAnswerOperationRef.current ||
      sessionListOperationRef.current ||
      sessionCurrentOperationRef.current
    ) {
      return;
    }
    const workspaceHandle = studio.handle;
    const request = ++sessionAnswerRequestRef.current;
    sessionAnswerOperationRef.current = true;
    setSessionAnswerBusy(true);
    setError(null);
    try {
      const answered = await window.motivo.sessions.answer({ ...input, workspaceHandle });
      if (
        request !== sessionAnswerRequestRef.current ||
        workspaceHandleRef.current !== workspaceHandle ||
        selectedSessionIdRef.current !== input.sessionId
      ) {
        return;
      }
      setSession(answered);
      setSessionList((list) =>
        list
          ? {
              ...list,
              sessions: list.sessions.map((item) =>
                item.sessionId === answered.sessionId ? answered : item,
              ),
            }
          : list,
      );
    } catch (caught) {
      if (
        request !== sessionAnswerRequestRef.current ||
        workspaceHandleRef.current !== workspaceHandle ||
        selectedSessionIdRef.current !== input.sessionId
      ) {
        return;
      }
      if (
        studioErrorCode(caught) === "session_turn_stale" ||
        studioErrorCode(caught) === "session_state_invalid"
      ) {
        await loadSessionCurrent(input.sessionId, workspaceHandle, true);
      } else {
        setError(errorMessage(caught));
      }
    } finally {
      if (
        request === sessionAnswerRequestRef.current &&
        workspaceHandleRef.current === workspaceHandle
      ) {
        sessionAnswerOperationRef.current = false;
        setSessionAnswerBusy(false);
      }
    }
  }

  const sharedWorkspaceActions = {
    onOpen: () => void chooseWorkspace("open"),
    onInitialize: () => void chooseWorkspace("initialize"),
  };

  return (
    <main className="studio-app">
      <StudioHeader
        studio={studio}
        workspaceBusy={workspaceBusy}
        actionBusy={
          actionBusy || sessionAnswerBusy || taskController.operating || taskController.running
        }
        {...sharedWorkspaceActions}
        onRefresh={() => void refreshWorkspace()}
      />
      <StudioSidebar
        view={view}
        tasks={taskController.tasks.length}
        scripts={snapshot?.scripts.length}
        plugins={pluginCount}
        runs={snapshot?.runs.length}
        sessions={sessionList?.sessions.length}
        onNavigate={setView}
      />

      <section className="main-stage">
        {loading ? (
          <LoadingView />
        ) : (
          <div className={`view-scroll ${activeAction ? "with-action" : ""}`}>
            {view === "tasks" ? (
              <TasksView
                key={studio?.handle ?? "no-workspace"}
                studio={studio}
                tasks={taskController.tasks}
                task={taskController.task}
                selectedId={taskController.selectedId}
                loading={taskController.loading}
                operating={taskController.operating}
                blocked={actionBusy || sessionAnswerBusy || workspaceBusy}
                anyRunning={taskController.running}
                onSelect={taskController.select}
                onCreate={(input) => taskController.mutate("create", input)}
                onContinue={(maxCalls, note) =>
                  taskController.mutate("continue", { maxCalls, ...(note ? { note } : {}) })
                }
                onPause={() => void taskController.mutate("pause", {})}
                {...sharedWorkspaceActions}
              />
            ) : view === "overview" ? (
              <OverviewView
                studio={studio}
                running={actionControlsBusy}
                workspaceBusy={workspaceBusy}
                {...sharedWorkspaceActions}
                onNavigate={setView}
                onAction={(request) => void startAction(request)}
              />
            ) : view === "workflow" ? (
              <WorkflowView
                key={studio?.handle ?? "no-workspace"}
                studio={studio}
                goal={goal}
                provider={provider}
                running={actionControlsBusy}
                onGoal={setGoal}
                onProvider={setProvider}
                onGenerate={() =>
                  void startAction({
                    kind: "generate",
                    goal: goal.trim(),
                    ...(provider ? { provider } : {}),
                  })
                }
                onCheck={(scripts) => void startAction({ kind: "check", scripts })}
                onRun={(scripts) => void startAction({ kind: "run", scripts })}
              />
            ) : view === "plugins" ? (
              <PluginsView
                studio={studio}
                running={actionControlsBusy}
                onSmoke={(plugin, live) =>
                  void startAction({
                    kind: "smoke",
                    targets: [{ namespace: plugin.namespace, name: plugin.name }],
                    live,
                  })
                }
              />
            ) : view === "runs" ? (
              <RunsView
                studio={studio}
                selectedRun={selectedRun}
                selectedRunId={selectedRunId}
                page={eventPage}
                events={events}
                busy={eventsBusy}
                onSelect={selectRun}
                onLoadMore={() => {
                  if (selectedRunId && eventPage) {
                    void loadEvents(selectedRunId, eventPage.nextAfter, true);
                  }
                }}
              />
            ) : (
              <SessionsView
                studio={studio}
                sessions={sessionList?.sessions ?? null}
                session={session}
                selectedSessionId={selectedSessionId}
                busy={sessionListBusy || sessionCurrentBusy}
                answering={sessionAnswerBusy}
                actionBusy={actionBusy || taskBusy}
                onSelect={selectSession}
                onReload={() => {
                  if (studio) void loadSessionList(studio.handle);
                }}
                onAnswer={(input) => void answerSession(input)}
              />
            )}
          </div>
        )}

        {activeAction ? (
          <ActionConsole
            action={activeAction}
            stream={outputStream}
            onStream={setOutputStream}
            onCancel={() => void cancelAction()}
            onClose={() => {
              if (!isActionBusy(activeAction)) setActiveAction(null);
            }}
          />
        ) : null}
        {error ? <ErrorToast error={error} onDismiss={() => setError(null)} /> : null}
      </section>
    </main>
  );
}

function studioErrorCode(error: unknown): string | undefined {
  if (typeof error !== "object" || error === null || !("detail" in error)) return undefined;
  const detail = error.detail;
  if (typeof detail !== "object" || detail === null || !("code" in detail)) return undefined;
  return typeof detail.code === "string" ? detail.code : undefined;
}
