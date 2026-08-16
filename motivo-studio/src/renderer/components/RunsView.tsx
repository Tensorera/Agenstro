import type { StudioEvent, StudioEventPage, StudioRun, StudioView } from "../../shared/contracts";
import { formatDateTime, formatEventData, formatTime, runState, runTone } from "../format";
import { Icon } from "./Icon";
import { PanelHeader, ViewHeader } from "./Primitives";

interface RunsViewProps {
  readonly studio: StudioView | null;
  readonly selectedRun: StudioRun | null;
  readonly selectedRunId: string | null;
  readonly page: StudioEventPage | null;
  readonly events: readonly StudioEvent[];
  readonly busy: boolean;
  readonly onSelect: (runId: string) => void;
  readonly onLoadMore: () => void;
}

export function RunsView({
  studio,
  selectedRun,
  selectedRunId,
  page,
  events,
  busy,
  onSelect,
  onLoadMore,
}: RunsViewProps) {
  const runs = studio?.snapshot.runs ?? [];
  return (
    <>
      <ViewHeader
        eyebrow="Factual evidence"
        title="Runs"
        description="Browse bounded, redacted projections of local Tactus journals. Event payloads are evidence, not deterministic replay state."
      />
      <div className="content-width runs-layout">
        <section className="run-browser" aria-label="Recent runs">
          <PanelHeader title="Run history" subtitle={`${runs.length} recent invocations`} />
          {runs.length === 0 ? (
            <div className="empty-list">No local run journals found.</div>
          ) : (
            <ul className="run-list">
              {runs.map((run) => (
                <li key={run.runId}>
                  <button
                    type="button"
                    className={`run-row ${selectedRunId === run.runId ? "selected" : ""}`}
                    aria-pressed={selectedRunId === run.runId}
                    onClick={() => onSelect(run.runId)}
                  >
                    <strong>{run.label}</strong>
                    <span className={`pill ${runTone(run)}`}>{runState(run)}</span>
                    <small>
                      {formatDateTime(run.startedUnixMs)} · {run.eventsRecorded} events
                    </small>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="event-browser" aria-label="Run events">
          <PanelHeader
            title={selectedRun?.label ?? "Select a run"}
            subtitle={selectedRun ? selectedRun.runId : "No trace selected"}
            action={
              selectedRun ? (
                <span className={`pill ${selectedRun.integrity === "ok" ? "success" : "warning"}`}>
                  {selectedRun.integrity}
                </span>
              ) : undefined
            }
          />
          {events.length === 0 ? (
            <div className="empty-list">
              {busy ? "Loading trace events…" : "This page contains no events."}
            </div>
          ) : (
            <ol className="event-list">
              {events.map((event) => (
                <li className="event-row" key={`${selectedRunId ?? "run"}:${event.seq}`}>
                  <span>#{event.seq}</span>
                  <strong className="event-kind" title={event.kind}>
                    {event.kind}
                  </strong>
                  <pre className="event-payload">{formatEventData(event.data)}</pre>
                  <time className="event-time">{formatTime(event.atUnixMs)}</time>
                </li>
              ))}
            </ol>
          )}
          <div className="load-more">
            <button
              type="button"
              className="button compact ghost"
              disabled={!page || page.complete || busy}
              onClick={onLoadMore}
            >
              <Icon name={busy ? "refresh" : "down"} />
              {busy ? "Loading…" : page?.complete ? "End of trace" : "Load more events"}
            </button>
          </div>
        </section>
      </div>
    </>
  );
}
