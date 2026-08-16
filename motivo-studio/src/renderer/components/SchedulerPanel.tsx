import { useEffect, useState } from "react";
import type { SchedulePage } from "../../shared/contracts";

interface SchedulerPanelProps {
  readonly available: boolean;
}

export function SchedulerPanel({ available }: SchedulerPanelProps) {
  const [page, setPage] = useState<SchedulePage | null>(null);
  const [cursor, setCursor] = useState<string | undefined>();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!available) return;
    let cancelled = false;
    void window.motivo.schedules
      .listPage({ pageSize: 50 })
      .then((value) => {
        if (!cancelled) setPage(value);
      })
      .catch((caught: unknown) => {
        if (!cancelled)
          setError(caught instanceof Error ? caught.message : "Scheduler unavailable");
      });
    return () => {
      cancelled = true;
    };
  }, [available]);

  async function load(cursorValue: string | undefined): Promise<void> {
    try {
      const value = await window.motivo.schedules.listPage({
        pageSize: 50,
        cursor: cursorValue,
      });
      setCursor(cursorValue);
      setPage(value);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Scheduler unavailable");
    }
  }

  async function next(): Promise<void> {
    if (!page?.nextCursor) return;
    await load(page.nextCursor);
  }

  return (
    <section className="scheduler-panel" aria-label="Scheduler view">
      <div className="panel-heading">
        <span>SCHEDULER / PAGE</span>
        <strong>{available ? "SEGNOD" : "OFFLINE"}</strong>
      </div>
      <div className="card-list">
        {page?.schedules.map((schedule) => (
          <article key={schedule.id} className="data-card">
            <span>{schedule.state}</span>
            <h2>{schedule.label}</h2>
            <code>{schedule.cron}</code>
            <p>{schedule.timezone}</p>
            <small>{schedule.nextFireAt ?? "No next occurrence"}</small>
          </article>
        ))}
        {!available ? (
          <p className="panel-empty">Background service discovery is unavailable.</p>
        ) : null}
        {error ? <p className="panel-error">{error}</p> : null}
      </div>
      <div className="pager">
        <button type="button" disabled={cursor === undefined} onClick={() => void load(undefined)}>
          First
        </button>
        <button type="button" disabled={!page?.nextCursor} onClick={() => void next()}>
          Next
        </button>
      </div>
    </section>
  );
}
