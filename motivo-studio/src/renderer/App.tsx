import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ActionRequest,
  StudioActionEvent,
  StudioEvent,
  StudioEventPage,
  StudioView,
} from "../shared/contracts";
import { ActionConsole } from "./components/ActionConsole";
import { OverviewView } from "./components/OverviewView";
import { PluginsView } from "./components/PluginsView";
import { RunsView } from "./components/RunsView";
import { ErrorToast, LoadingView, StudioHeader, StudioSidebar } from "./components/StudioChrome";
import { WorkflowView } from "./components/WorkflowView";
import { appendBounded, errorMessage } from "./format";
import {
  EVENT_PAGE_SIZE,
  isActionBusy,
  type ActiveAction,
  type NavigationView,
  type OutputStream,
} from "./model";

export default function App() {
  const bridgeAvailable = "motivo" in window;
  const [view, setView] = useState<NavigationView>("overview");
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
  const actionIdRef = useRef<string | null>(null);
  const eventRequestRef = useRef(0);

  const acceptStudio = useCallback((next: StudioView | null) => {
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
          };
        });
        return;
      }

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

  useEffect(() => {
    if (view !== "runs" || !selectedRunId) return;
    void loadEvents(selectedRunId, "0", false);
  }, [selectedRunId, view]);

  async function chooseWorkspace(operation: "open" | "initialize"): Promise<void> {
    if (!("motivo" in window)) return;
    setWorkspaceBusy(true);
    setError(null);
    try {
      const selected =
        operation === "open"
          ? await window.motivo.studio.openInitialized()
          : await window.motivo.studio.initialize();
      if (selected) {
        acceptStudio(selected);
        setView("overview");
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
    if (!studio) return;
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
    if (!studio || isActionBusy(activeAction)) return;
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
            },
      );
      setOutputStream("stdout");
    } catch (caught) {
      setError(errorMessage(caught));
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

  const running = isActionBusy(activeAction);
  const sharedWorkspaceActions = {
    onOpen: () => void chooseWorkspace("open"),
    onInitialize: () => void chooseWorkspace("initialize"),
  };

  return (
    <main className="studio-app">
      <StudioHeader
        studio={studio}
        workspaceBusy={workspaceBusy}
        actionBusy={running}
        {...sharedWorkspaceActions}
        onRefresh={() => void refreshWorkspace()}
      />
      <StudioSidebar
        view={view}
        scripts={snapshot?.scripts.length}
        plugins={pluginCount}
        runs={snapshot?.runs.length}
        onNavigate={setView}
      />

      <section className="main-stage">
        {loading ? (
          <LoadingView />
        ) : (
          <div className={`view-scroll ${activeAction ? "with-action" : ""}`}>
            {view === "overview" ? (
              <OverviewView
                studio={studio}
                running={running}
                workspaceBusy={workspaceBusy}
                {...sharedWorkspaceActions}
                onNavigate={setView}
                onAction={(request) => void startAction(request)}
              />
            ) : view === "workflow" ? (
              <WorkflowView
                studio={studio}
                goal={goal}
                provider={provider}
                running={running}
                onGoal={setGoal}
                onProvider={setProvider}
                onGenerate={() =>
                  void startAction({
                    kind: "generate",
                    goal: goal.trim(),
                    ...(provider ? { provider } : {}),
                  })
                }
                onCheck={() => void startAction({ kind: "check" })}
                onRun={() => void startAction({ kind: "run" })}
              />
            ) : view === "plugins" ? (
              <PluginsView
                studio={studio}
                running={running}
                onSmoke={(plugin, live) =>
                  void startAction({
                    kind: "smoke",
                    targets: [{ namespace: plugin.namespace, name: plugin.name }],
                    live,
                  })
                }
              />
            ) : (
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
