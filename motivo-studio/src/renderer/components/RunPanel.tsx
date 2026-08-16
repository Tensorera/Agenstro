import type { Run, RunEvent } from "../../shared/contracts";

interface RunPanelProps {
  readonly run: Run | null;
  readonly events: readonly RunEvent[];
  onCancel(): void;
}

export function RunPanel({ run, events, onCancel }: RunPanelProps) {
  const active = run?.state === "queued" || run?.state === "running" || run?.state === "recovering";
  return (
    <section className="run-panel" aria-label="Run events">
      <div className="pane-title">
        <span>RUN / RESUMABLE EVENTS</span>
        <strong>{run?.state ?? "idle"}</strong>
        <button type="button" disabled={!active} onClick={onCancel}>
          Cancel
        </button>
      </div>
      <ol className="event-list">
        {events.map((event) => (
          <li key={event.sequence}>
            <span>{event.sequence.padStart(4, "0")}</span>
            <strong>{event.body.kind}</strong>
            <p>{eventText(event)}</p>
          </li>
        ))}
        {!events.length ? (
          <li className="empty-event">Accepted runs stream here from the last sequence.</li>
        ) : null}
      </ol>
    </section>
  );
}

function eventText(event: RunEvent): string {
  switch (event.body.kind) {
    case "started":
      return event.body.label;
    case "stage":
      return `${event.body.label} / ${event.body.state}`;
    case "output":
      return event.body.data;
    case "diagnostic":
      return `${event.body.code}: ${event.body.message}`;
    case "finished":
      return event.body.summary ?? event.body.state;
  }
}
