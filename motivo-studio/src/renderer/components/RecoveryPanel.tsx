import { useEffect, useState } from "react";
import {
  requestIdSchema,
  type RecoveryPage,
  type Workspace,
  type WorkspaceId,
} from "../../shared/contracts";

interface RecoveryPanelProps {
  readonly workspace: Workspace | null;
}

export function RecoveryPanel({ workspace }: RecoveryPanelProps) {
  const [result, setResult] = useState<{
    readonly workspaceId: WorkspaceId;
    readonly page: RecoveryPage;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const workspaceId = workspace?.id;
  const page = result && result.workspaceId === workspaceId ? result.page : null;

  useEffect(() => {
    if (!workspaceId) return;
    let cancelled = false;
    void window.motivo.recovery
      .listPage({ workspaceId, pageSize: 50 })
      .then((value) => {
        if (!cancelled) setResult({ workspaceId, page: value });
      })
      .catch((caught: unknown) => {
        if (!cancelled) setError(caught instanceof Error ? caught.message : "Recovery unavailable");
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  async function apply(record: RecoveryPage["records"][number]): Promise<void> {
    if (!workspace || !window.confirm(`Apply recovery "${record.label}"?`)) return;
    try {
      const updated = await window.motivo.recovery.apply({
        requestId: requestIdSchema.parse(crypto.randomUUID()),
        workspaceId: workspace.id,
        recoveryId: record.id,
      });
      setResult((current) =>
        current && current.workspaceId === workspace.id
          ? {
              ...current,
              page: {
                ...current.page,
                records: current.page.records.map((candidate) =>
                  candidate.id === updated.id ? updated : candidate,
                ),
              },
            }
          : current,
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Recovery failed");
    }
  }

  return (
    <section className="recovery-panel" aria-label="Recovery view">
      <div className="panel-heading">
        <span>RECOVERY / PAGE</span>
        <strong>{workspace ? "TACTUSD" : "NO WORKSPACE"}</strong>
      </div>
      <div className="card-list">
        {page?.records.map((record) => (
          <article key={record.id} className="data-card">
            <span>{record.state}</span>
            <h2>{record.label}</h2>
            <p>{record.changedFiles} changed files</p>
            <small>{new Date(record.createdAt).toLocaleString()}</small>
            <button
              type="button"
              disabled={record.state !== "available"}
              onClick={() => void apply(record)}
            >
              Apply
            </button>
          </article>
        ))}
        {!workspace ? (
          <p className="panel-empty">Open a workspace to inspect recovery records.</p>
        ) : null}
        {error ? <p className="panel-error">{error}</p> : null}
      </div>
    </section>
  );
}
