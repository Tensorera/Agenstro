import { useEffect, useRef, useState } from "react";
import {
  requestIdSchema,
  sequenceSchema,
  type EntryId,
  type FileDocument,
  type FilePage,
  type Run,
  type RunEvent,
  type RunStreamHandle,
  type StudioSnapshot,
  type Workspace,
} from "../shared/contracts";
import type { StudioSurface } from "../shared/surface";
import { EditorPane } from "./editor/EditorPane";
import { FileExplorer } from "./components/FileExplorer";
import { RecoveryPanel } from "./components/RecoveryPanel";
import { RunPanel } from "./components/RunPanel";
import { SchedulerPanel } from "./components/SchedulerPanel";
import { TerminalPane } from "./components/TerminalPane";

type NavigationView = StudioSurface | "recovery";

interface DirectoryFrame {
  readonly id: EntryId;
  readonly name: string;
}

export default function App() {
  const [snapshot, setSnapshot] = useState<StudioSnapshot | null>(null);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [view, setView] = useState<NavigationView>("files");
  const [directoryStack, setDirectoryStack] = useState<readonly DirectoryFrame[]>([]);
  const [page, setPage] = useState<FilePage | null>(null);
  const [pageCursor, setPageCursor] = useState<string | undefined>();
  const [document, setDocument] = useState<FileDocument | null>(null);
  const [content, setContent] = useState("");
  const [run, setRun] = useState<Run | null>(null);
  const [runEvents, setRunEvents] = useState<readonly RunEvent[]>([]);
  const [streamStart, setStreamStart] = useState<{
    readonly runId: Run["id"];
    readonly afterSequence: Run["lastSequence"];
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(
    "motivo" in window ? null : "The secure Electron preload bridge is unavailable.",
  );
  const lastRunSequence = useRef("0");

  useEffect(() => {
    let cancelled = false;
    if (!("motivo" in window)) return;
    void window.motivo.system
      .snapshot()
      .then((value) => {
        if (!cancelled) setSnapshot(value);
      })
      .catch((caught: unknown) => {
        if (!cancelled) setError(errorMessage(caught));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!("motivo" in window)) return;
    let cancelled = false;
    let receivedUpdate = false;
    const unsubscribe = window.motivo.surface.subscribe((surface) => {
      receivedUpdate = true;
      if (!cancelled) setView(surface);
    });
    void window.motivo.surface
      .current()
      .then((surface) => {
        if (!cancelled && !receivedUpdate) setView(surface);
      })
      .catch((caught: unknown) => {
        if (!cancelled) setError(errorMessage(caught));
      });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    if (!streamStart) return;
    let disposed = false;
    let handle: RunStreamHandle | undefined;
    lastRunSequence.current = streamStart.afterSequence;
    const resync = async () => {
      try {
        const current = await window.motivo.runs.get({ runId: streamStart.runId });
        if (disposed) return;
        setRun(current);
        setRunEvents([]);
        lastRunSequence.current = current.lastSequence;
        setStreamStart({ runId: current.id, afterSequence: current.lastSequence });
      } catch (caught) {
        if (!disposed) setError(errorMessage(caught));
      }
    };
    void window.motivo.runs
      .subscribe(
        { runId: streamStart.runId, afterSequence: streamStart.afterSequence },
        (message) => {
          if (disposed) return;
          if (message.kind === "events") {
            for (const event of message.events) {
              const expected = BigInt(lastRunSequence.current) + 1n;
              if (BigInt(event.sequence) !== expected) {
                void resync();
                return;
              }
              lastRunSequence.current = event.sequence;
              setRunEvents((current) => [...current.slice(-199), event]);
              if (event.body.kind === "finished") {
                const finished = event.body;
                setRun((current) =>
                  current
                    ? {
                        ...current,
                        state: finished.state,
                        lastSequence: event.sequence,
                        detail: finished.summary,
                      }
                    : current,
                );
              } else {
                setRun((current) =>
                  current ? { ...current, lastSequence: event.sequence } : current,
                );
              }
            }
            const highest = message.events.at(-1)?.sequence;
            if (highest && handle) void handle.ack(highest).catch(() => undefined);
          } else if (message.kind === "resync-required") {
            void resync();
          } else if (message.error) {
            setError(message.error.message);
          }
        },
      )
      .then((subscription) => {
        if (disposed) {
          void subscription.unsubscribe().catch(() => undefined);
          return;
        }
        handle = subscription;
        if (lastRunSequence.current !== streamStart.afterSequence) {
          void subscription
            .ack(sequenceSchema.parse(lastRunSequence.current))
            .catch(() => undefined);
        }
      })
      .catch((caught: unknown) => {
        if (!disposed) setError(errorMessage(caught));
      });
    return () => {
      disposed = true;
      if (handle) void handle.unsubscribe().catch(() => undefined);
    };
  }, [streamStart]);

  async function openWorkspace(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      const selected = await window.motivo.workspaces.open();
      if (!selected) return;
      setWorkspace(selected);
      setDocument(null);
      setRun(null);
      setRunEvents([]);
      setStreamStart(null);
      lastRunSequence.current = "0";
      const root = { id: selected.rootEntryId, name: selected.name };
      setDirectoryStack([root]);
      await loadDirectory(selected, [root], undefined);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  async function loadDirectory(
    activeWorkspace: Workspace,
    stack: readonly DirectoryFrame[],
    cursor: string | undefined,
  ): Promise<void> {
    const directory = stack.at(-1);
    if (!directory) return;
    setBusy(true);
    try {
      const nextPage = await window.motivo.files.listPage({
        workspaceId: activeWorkspace.id,
        parentId: directory.id,
        pageSize: 80,
        cursor,
      });
      setPage(nextPage);
      setPageCursor(cursor);
      setDirectoryStack(stack.slice(-64));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  async function selectEntry(entry: FilePage["entries"][number]): Promise<void> {
    if (!workspace) return;
    if (entry.kind === "directory") {
      await loadDirectory(
        workspace,
        [...directoryStack, { id: entry.id, name: entry.name }],
        undefined,
      );
      return;
    }
    setBusy(true);
    try {
      const selected = await window.motivo.files.read({
        workspaceId: workspace.id,
        entryId: entry.id,
      });
      setDocument(selected);
      setContent(selected.content);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  async function saveFile(): Promise<void> {
    if (!document || content === document.content) return;
    setBusy(true);
    try {
      const saved = await window.motivo.files.save({
        requestId: newRequestId(),
        workspaceId: document.workspaceId,
        entryId: document.entryId,
        expectedRevision: document.revision,
        content,
      });
      setDocument(saved);
      setContent(saved.content);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  async function startRun(): Promise<void> {
    if (!workspace || !document) return;
    setBusy(true);
    setRunEvents([]);
    try {
      const accepted = await window.motivo.runs.start({
        requestId: newRequestId(),
        workspaceId: workspace.id,
        entryId: document.entryId,
      });
      setRun(accepted);
      setStreamStart({ runId: accepted.id, afterSequence: accepted.lastSequence });
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  async function cancelRun(): Promise<void> {
    if (!run) return;
    try {
      setRun(await window.motivo.runs.cancel({ requestId: newRequestId(), runId: run.id }));
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  const dirty = document !== null && content !== document.content;
  const daemonTone = snapshot?.state ?? "starting";

  return (
    <main className="studio-shell">
      <header className="titlebar">
        <div className="wordmark" aria-label="Motivo Studio">
          <span className="motif" aria-hidden="true" />
          <span>
            <strong>MOTIVO</strong>
            <small>STUDIO</small>
          </span>
        </div>
        <div className="workspace-title">
          <span>{workspace?.name ?? "No workspace"}</span>
          <small>{snapshot?.state === "ready" ? "LOCAL CONTROL PLANE" : "DEGRADED MODE"}</small>
        </div>
        <span className={`daemon-indicator ${daemonTone}`}>{daemonTone}</span>
        <button
          type="button"
          className="open-button"
          disabled={busy}
          onClick={() => void openWorkspace()}
        >
          Open workspace
        </button>
      </header>

      <div className="studio-grid">
        <aside className="activity-rail" aria-label="Studio views">
          <button
            type="button"
            className={view === "files" ? "active" : ""}
            aria-label="Files"
            onClick={() => setView("files")}
          >
            FI
          </button>
          <button
            type="button"
            className={view === "scheduler" ? "active" : ""}
            aria-label="Scheduler"
            onClick={() => setView("scheduler")}
          >
            SC
          </button>
          <button
            type="button"
            className={view === "recovery" ? "active" : ""}
            aria-label="Recovery"
            onClick={() => setView("recovery")}
          >
            RC
          </button>
        </aside>

        <aside className="side-panel">
          {view === "files" ? (
            <FileExplorer
              workspace={workspace}
              page={page}
              stack={directoryStack}
              busy={busy}
              onSelect={(entry) => void selectEntry(entry)}
              onUp={() => {
                if (workspace && directoryStack.length > 1) {
                  void loadDirectory(workspace, directoryStack.slice(0, -1), undefined);
                }
              }}
              onNext={() => {
                if (workspace && page?.nextCursor) {
                  void loadDirectory(workspace, directoryStack, page.nextCursor);
                }
              }}
              onFirst={() => {
                if (workspace) void loadDirectory(workspace, directoryStack, undefined);
              }}
              pageCursor={pageCursor}
            />
          ) : view === "scheduler" ? (
            <SchedulerPanel available={snapshot?.services[2]?.state === "ready"} />
          ) : (
            <RecoveryPanel workspace={workspace} />
          )}
        </aside>

        <section className="workbench">
          <section className="editor-section">
            <div className="editor-header">
              <span className={dirty ? "file-state dirty" : "file-state"} aria-hidden="true" />
              <strong>{document?.name ?? "Select a workspace file"}</strong>
              {document?.truncated ? <span className="tag warning">TRUNCATED</span> : null}
              {document?.binary ? <span className="tag warning">BINARY</span> : null}
              <div className="editor-actions">
                <button
                  type="button"
                  disabled={!dirty || busy || document?.readOnly}
                  onClick={() => void saveFile()}
                >
                  Save
                </button>
                <button
                  type="button"
                  className="run-button"
                  disabled={!document || busy || document.binary || document.truncated}
                  onClick={() => void startRun()}
                >
                  Run file
                </button>
              </div>
            </div>
            {workspace && document && !document.binary && !document.truncated ? (
              <EditorPane
                workspaceId={workspace.id}
                entryId={document.entryId}
                path={document.name}
                revision={document.revision}
                language={document.language}
                value={content}
                readOnly={busy || document.readOnly}
                onChange={setContent}
              />
            ) : (
              <div className="empty-editor">
                <span>BOUNDARY / 01</span>
                <h1>{workspace ? "Choose a text file" : "Connect a workspace"}</h1>
                <p>
                  Files are paged by tactusd. The renderer never receives a host path, daemon token,
                  or general filesystem capability.
                </p>
              </div>
            )}
          </section>
          <RunPanel run={run} events={runEvents} onCancel={() => void cancelRun()} />
          <TerminalPane workspace={workspace} />
        </section>
      </div>

      <footer className="statusbar">
        <span>{busy ? "operation pending" : "ready"}</span>
        <span>{error ?? "IPC allowlist active / renderer sandboxed"}</span>
        <span>{workspace ? `revision ${workspace.revision}` : "no workspace"}</span>
      </footer>
    </main>
  );
}

function newRequestId() {
  return requestIdSchema.parse(crypto.randomUUID());
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "The operation failed.";
}
