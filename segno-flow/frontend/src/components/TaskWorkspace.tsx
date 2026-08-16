import { Icon } from "./Icon";
import { StatusPill } from "./StatusPill";
import {
  fileName,
  formatBytes,
  formatDateTime,
  formatDuration,
  formatRelative,
  sentenceCase,
} from "../lib/format";
import type {
  PhaseName,
  SystemStatus,
  TaskRunDetail,
  TaskRunSummary,
  TaskSummary,
} from "../types/segnoFlow";

interface TaskWorkspaceProps {
  task: TaskSummary;
  system: SystemStatus | null;
  runs: TaskRunSummary[];
  selectedRunId: string | null;
  runDetail: TaskRunDetail | null;
  runsLoading: boolean;
  detailLoading: boolean;
  runningNow: boolean;
  toggling: boolean;
  onRunNow: () => void;
  onToggle: () => void;
  onSelectRun: (runId: string) => void;
}

function scriptLabel(task: TaskSummary, phase: PhaseName): string {
  const value = task.scripts[phase];
  if (value) return fileName(value);
  if (phase === "main") return "main.py";
  return "Optional";
}

function TaskHeader({
  task,
  runningNow,
  toggling,
  onRunNow,
  onToggle,
}: Pick<
  TaskWorkspaceProps,
  "task" | "runningNow" | "toggling" | "onRunNow" | "onToggle"
>) {
  return (
    <header className="task-header">
      <div className="task-title-block">
        <div className="task-title-line">
          <h1>{task.name}</h1>
          <StatusPill status={task.status} />
        </div>
        <p>{task.description || "No description was supplied with this task package."}</p>
      </div>
      <div className="task-actions">
        <button
          type="button"
          className="toggle-button"
          aria-pressed={task.enabled}
          aria-label={task.enabled ? "Pause schedule" : "Enable schedule"}
          disabled={toggling}
          onClick={onToggle}
        >
          <span className="toggle-track" aria-hidden="true"><span /></span>
          {toggling ? "Saving…" : task.enabled ? "Enabled" : "Paused"}
        </button>
        <button
          type="button"
          className="primary-button"
          disabled={runningNow || task.status === "running"}
          onClick={onRunNow}
        >
          <Icon name={runningNow || task.status === "running" ? "clock" : "play"} />
          {runningNow ? "Starting…" : task.status === "running" ? "Running" : "Run now"}
        </button>
      </div>
    </header>
  );
}

function ScheduleCard({ task }: { task: TaskSummary }) {
  return (
    <section className="summary-card schedule-card" aria-labelledby="schedule-heading">
      <div className="card-heading">
        <span className="card-icon"><Icon name="calendar" /></span>
        <div>
          <p className="eyebrow">Schedule</p>
          <h2 id="schedule-heading">{task.enabled ? "Automatic runs active" : "Schedule paused"}</h2>
        </div>
      </div>
      <div className="cron-display">
        <code>{task.cron}</code>
        <span>{task.timezone}</span>
      </div>
      <dl className="summary-pairs">
        <div>
          <dt>Next run</dt>
          <dd>{task.enabled ? formatDateTime(task.nextRunAt) : "Paused"}</dd>
        </div>
        <div>
          <dt>Last run</dt>
          <dd title={task.lastRunAt ?? undefined}>{formatRelative(task.lastRunAt)}</dd>
        </div>
      </dl>
    </section>
  );
}

function LocationCard({ task, system }: { task: TaskSummary; system: SystemStatus | null }) {
  const taskDirectory = task.taskDirectory ||
    (system?.installationRoot ? `${system.installationRoot}\\tasks\\${task.id}` : "Managed by Segno Flow");
  return (
    <section className="summary-card location-card" aria-labelledby="location-heading">
      <div className="card-heading">
        <span className="card-icon"><Icon name="folder" /></span>
        <div>
          <p className="eyebrow">Storage</p>
          <h2 id="location-heading">Working locations</h2>
        </div>
      </div>
      <dl className="path-list">
        <div>
          <dt>Target directory</dt>
          <dd title={task.targetDirectory}>{task.targetDirectory || "Not configured"}</dd>
        </div>
        <div>
          <dt>Task files</dt>
          <dd title={taskDirectory}>{taskDirectory}</dd>
        </div>
      </dl>
    </section>
  );
}

function PipelineCard({ task }: { task: TaskSummary }) {
  const pipeline: Array<{ phase: PhaseName; label: string; hint: string }> = [
    { phase: "preprocess", label: "Prepare", hint: "Collect inputs" },
    { phase: "main", label: "Execute", hint: `${task.scripts.helpers.length} helper${task.scripts.helpers.length === 1 ? "" : "s"}` },
    { phase: "postprocess", label: "Publish", hint: "Store artifacts" },
  ];
  return (
    <section className="pipeline-card" aria-labelledby="pipeline-heading">
      <div className="section-heading-row">
        <div>
          <p className="eyebrow">Workflow</p>
          <h2 id="pipeline-heading">Execution pipeline</h2>
        </div>
        <span className="package-badge"><Icon name="archive" /> Imported package</span>
      </div>
      <ol className="pipeline">
        {pipeline.map((item, index) => (
          <li key={item.phase} className={!task.scripts[item.phase] && item.phase !== "main" ? "optional" : ""}>
            <span className="step-number">0{index + 1}</span>
            <div>
              <strong>{item.label}</strong>
              <code>{scriptLabel(task, item.phase)}</code>
              <span>{item.hint}</span>
            </div>
            {index < pipeline.length - 1 ? <Icon name="chevron" className="pipeline-arrow" /> : null}
          </li>
        ))}
      </ol>
    </section>
  );
}

function RunHistory({
  runs,
  selectedRunId,
  loading,
  onSelect,
}: {
  runs: TaskRunSummary[];
  selectedRunId: string | null;
  loading: boolean;
  onSelect: (runId: string) => void;
}) {
  return (
    <section className="history-panel" aria-labelledby="history-heading">
      <div className="section-heading-row compact-heading">
        <div>
          <p className="eyebrow">Activity</p>
          <h2 id="history-heading">Run history</h2>
        </div>
        <span className="history-count">{runs.length}</span>
      </div>
      <div className="run-list" aria-busy={loading}>
        {loading ? (
          <div className="inline-loader" role="status"><span /> Loading history…</div>
        ) : runs.length ? (
          runs.map((run) => (
            <button
              type="button"
              key={run.id}
              className={`run-row ${selectedRunId === run.id ? "selected" : ""}`}
              aria-current={selectedRunId === run.id ? "true" : undefined}
              onClick={() => onSelect(run.id)}
            >
              <span className={`run-icon status-${run.status}`} aria-hidden="true">
                {run.status === "succeeded" ? <Icon name="check" /> : ["failed", "timed_out", "interrupted"].includes(run.status) ? <Icon name="warning" /> : <Icon name="clock" />}
              </span>
              <span className="run-main">
                <span><strong>{formatDateTime(run.startedAt)}</strong><small>{sentenceCase(run.trigger)}</small></span>
                <span className="run-summary">{run.summary || sentenceCase(run.status)}</span>
              </span>
              <span className="run-meta">
                <StatusPill status={run.status} compact />
                <small>{formatDuration(run.durationMs)}</small>
              </span>
              <Icon name="chevron" className="row-chevron" />
            </button>
          ))
        ) : (
          <div className="panel-empty">
            <Icon name="history" />
            <strong>No runs yet</strong>
            <span>Use Run now or wait for the next scheduled time.</span>
          </div>
        )}
      </div>
    </section>
  );
}

function RunInspector({ detail, loading }: { detail: TaskRunDetail | null; loading: boolean }) {
  if (loading) {
    return <section className="run-inspector"><div className="inline-loader centered" role="status"><span /> Loading run details…</div></section>;
  }
  if (!detail) {
    return (
      <section className="run-inspector empty-inspector">
        <Icon name="history" />
        <strong>Select a run</strong>
        <span>Logs, phase results, and artifacts will appear here.</span>
      </section>
    );
  }
  return (
    <section className="run-inspector" aria-labelledby="run-detail-heading">
      <div className="inspector-heading">
        <div>
          <p className="eyebrow">Run detail</p>
          <h2 id="run-detail-heading">{formatDateTime(detail.startedAt)}</h2>
        </div>
        <StatusPill status={detail.status} />
      </div>

      {detail.error ? <div className="run-error"><Icon name="warning" />{detail.error}</div> : null}

      {detail.phases.length ? (
        <ol className="phase-track" aria-label="Run phases">
          {detail.phases.map((phase) => (
            <li key={phase.name} className={`status-${phase.status}`}>
              <span className="phase-marker" aria-hidden="true">
                {phase.status === "succeeded" ? <Icon name="check" /> : ["failed", "timed_out", "interrupted"].includes(phase.status) ? <Icon name="warning" /> : <span />}
              </span>
              <span><strong>{sentenceCase(phase.name)}</strong><small>{phase.exitCode === null ? sentenceCase(phase.status) : `Exit ${phase.exitCode}`}</small></span>
            </li>
          ))}
        </ol>
      ) : null}

      <div className="log-heading">
        <h3>Execution log</h3>
        <span>{detail.logs.length} lines</span>
      </div>
      <div className="log-view" tabIndex={0} aria-label="Execution log">
        {detail.logs.length ? detail.logs.map((entry, index) => (
          <div className={`log-line level-${entry.level}`} key={`${entry.timestamp}-${index}`}>
            <time>{new Date(entry.timestamp).toLocaleTimeString([], { hour12: false })}</time>
            <span className="log-phase">{entry.phase}</span>
            <span>{entry.message}</span>
          </div>
        )) : <span className="log-empty">No log output was recorded for this run.</span>}
      </div>

      {detail.artifacts.length ? (
        <div className="artifacts">
          <h3>Artifacts</h3>
          <div className="artifact-list">
            {detail.artifacts.map((artifact) => (
              <div key={artifact.path} className="artifact-row" title={artifact.path}>
                <Icon name="archive" />
                <span><strong>{artifact.name}</strong><small>{artifact.path}</small></span>
                <small>{formatBytes(artifact.size)}</small>
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
}

export function TaskWorkspace(props: TaskWorkspaceProps) {
  return (
    <div className="task-workspace">
      <TaskHeader {...props} />
      <div className="workspace-scroll">
        <div className="summary-grid">
          <ScheduleCard task={props.task} />
          <LocationCard task={props.task} system={props.system} />
        </div>
        <PipelineCard task={props.task} />
        <div className="run-grid">
          <RunHistory
            runs={props.runs}
            selectedRunId={props.selectedRunId}
            loading={props.runsLoading}
            onSelect={props.onSelectRun}
          />
          <RunInspector detail={props.runDetail} loading={props.detailLoading} />
        </div>
      </div>
    </div>
  );
}
