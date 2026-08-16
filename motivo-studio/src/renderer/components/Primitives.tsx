import type { ReactNode } from "react";
import type { StudioRun } from "../../shared/contracts";
import { formatDateTime, runState, runTone } from "../format";

export function ViewHeader({
  eyebrow,
  title,
  description,
  action,
}: {
  readonly eyebrow: string;
  readonly title: string;
  readonly description: string;
  readonly action?: ReactNode;
}) {
  return (
    <header className="view-header">
      <div>
        <p className="eyebrow">{eyebrow}</p>
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      {action}
    </header>
  );
}

export function PanelHeader({
  title,
  subtitle,
  action,
}: {
  readonly title: string;
  readonly subtitle: string;
  readonly action?: ReactNode;
}) {
  return (
    <header className="panel-header">
      <div>
        <h2>{title}</h2>
        <p>{subtitle}</p>
      </div>
      {action}
    </header>
  );
}

export function Metric({
  label,
  value,
  note,
}: {
  readonly label: string;
  readonly value: string | number;
  readonly note: string;
}) {
  return (
    <article className="metric-card">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </article>
  );
}

export function RunTable({
  runs,
  onSelect,
}: {
  readonly runs: readonly StudioRun[];
  readonly onSelect: (run: StudioRun) => void;
}) {
  if (runs.length === 0) return <div className="empty-list">No run journals recorded yet.</div>;
  return (
    <table className="data-table">
      <thead>
        <tr>
          <th>Invocation</th>
          <th>State</th>
          <th>Started</th>
          <th>Events</th>
        </tr>
      </thead>
      <tbody>
        {runs.map((run) => (
          <tr className="interactive" key={run.runId} onClick={() => onSelect(run)}>
            <td title={run.label}>{run.label}</td>
            <td>
              <span className={`pill ${runTone(run)}`}>{runState(run)}</span>
            </td>
            <td className="mono dim">{formatDateTime(run.startedUnixMs)}</td>
            <td className="mono dim">{run.eventsRecorded}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
