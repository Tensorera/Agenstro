import { useEffect, useState } from "react";
import type { StudioView } from "../../shared/contracts";
import type {
  TaskAction,
  TaskDocument,
  TaskReport,
  TaskRound,
  TaskSummary,
} from "../../shared/task-contracts";
import { Icon } from "./Icon";
import { PanelHeader, ViewHeader } from "./Primitives";

const actionLabels: Record<TaskAction, string> = {
  investigate: "Investigation",
  try: "Attempt",
  integrate: "Integration",
  conclude: "Conclusion",
};
const statusLabels: Record<TaskDocument["status"], string> = {
  ready: "Ready",
  running: "Working",
  paused: "Paused",
  needs_input: "Needs your input",
  completed: "Completed",
  failed: "Stopped after an error",
  outcome_unknown: "Outcome needs checking",
};

interface TasksViewProps {
  readonly studio: StudioView | null;
  readonly tasks: readonly TaskSummary[];
  readonly task: TaskDocument | null;
  readonly selectedId: string | null;
  readonly loading: boolean;
  readonly operating: boolean;
  readonly blocked: boolean;
  readonly anyRunning: boolean;
  readonly onSelect: (taskId: string) => void;
  readonly onCreate: (input: {
    goal: string;
    constraints: string;
    provider: string;
  }) => Promise<boolean>;
  readonly onContinue: (maxCalls: number, note?: string) => Promise<boolean>;
  readonly onPause: () => void;
  readonly onOpen: () => void;
  readonly onInitialize: () => void;
}

export function TasksView(props: TasksViewProps) {
  const { studio, tasks, task, loading, operating, blocked, anyRunning, selectedId, onSelect } =
    props;
  const [creating, setCreating] = useState(false);
  if (!studio) {
    return (
      <div className="content-width">
        <section className="empty-workspace" aria-label="No workspace">
          <div>
            <span className="empty-symbol">
              <Icon name="spark" />
            </span>
            <p className="eyebrow">A place to work things out</p>
            <h2>Start with the problem.</h2>
            <p>
              Investigate what matters, try an approach, and keep the findings as the work develops.
            </p>
            <div className="empty-actions">
              <button
                type="button"
                className="button primary"
                disabled={blocked}
                onClick={props.onOpen}
              >
                <Icon name="folder" /> Open workspace
              </button>
              <button
                type="button"
                className="button"
                disabled={blocked}
                onClick={props.onInitialize}
              >
                <Icon name="plus" /> Initialize folder
              </button>
            </div>
          </div>
        </section>
      </div>
    );
  }
  const showForm = creating || (!loading && tasks.length === 0);
  return (
    <>
      <ViewHeader
        eyebrow="Follow the evidence"
        title="Tasks"
        description="Keep the goal in view. Investigate, try, integrate, or conclude as the work calls for it."
      />
      <div className="content-width task-layout">
        <aside className="panel task-library" aria-label="Saved tasks">
          <PanelHeader
            title="Your work"
            subtitle={`${tasks.length} saved ${tasks.length === 1 ? "task" : "tasks"}`}
          />
          <div className="task-library-actions">
            <button
              type="button"
              className="button primary"
              disabled={operating || blocked || anyRunning}
              onClick={() => setCreating(true)}
            >
              <Icon name="plus" /> New task
            </button>
          </div>
          <div className="task-list">
            {tasks.map((item) => (
              <button
                type="button"
                className={`task-list-item ${!creating && selectedId === item.id ? "selected" : ""}`}
                key={item.id}
                aria-pressed={!creating && selectedId === item.id}
                disabled={operating}
                onClick={() => {
                  setCreating(false);
                  onSelect(item.id);
                }}
              >
                <span className={`task-status-dot ${item.status}`} aria-hidden="true" />
                <span>
                  <strong>{item.goal}</strong>
                  <small>
                    {statusLabels[item.status]} · {item.calls} agent{" "}
                    {item.calls === 1 ? "call" : "calls"}
                  </small>
                </span>
              </button>
            ))}
            {loading && tasks.length === 0 ? (
              <p className="task-empty" role="status">
                Loading tasks…
              </p>
            ) : null}
            {!loading && tasks.length === 0 ? (
              <p className="task-empty">Your goals and findings will stay here between visits.</p>
            ) : null}
          </div>
        </aside>
        <div className="task-main">
          {showForm ? (
            <NewTaskForm
              studio={studio}
              busy={operating || blocked || anyRunning}
              onCancel={tasks.length ? () => setCreating(false) : undefined}
              onCreate={async (input) => {
                const saved = await props.onCreate(input);
                if (saved) setCreating(false);
              }}
            />
          ) : task ? (
            <TaskDetail
              key={task.id}
              task={task}
              operating={operating}
              blocked={blocked}
              anotherRunning={anyRunning && task.status !== "running"}
              onContinue={props.onContinue}
              onPause={props.onPause}
            />
          ) : (
            <article className="panel">
              <p className="task-empty" role="status">
                {loading ? "Opening task…" : "Select a task to see its findings."}
              </p>
            </article>
          )}
        </div>
      </div>
    </>
  );
}

function NewTaskForm({
  studio,
  busy,
  onCreate,
  onCancel,
}: {
  readonly studio: StudioView;
  readonly busy: boolean;
  readonly onCreate: (input: {
    goal: string;
    constraints: string;
    provider: string;
  }) => Promise<void>;
  readonly onCancel: (() => void) | undefined;
}) {
  const [goal, setGoal] = useState("");
  const [constraints, setConstraints] = useState("");
  const [provider, setProvider] = useState(studio.snapshot.registries.defaultProvider);
  return (
    <article className="panel task-create">
      <PanelHeader
        title="What are you working toward?"
        subtitle="Describe the outcome and what a useful result would look like."
      />
      <form
        className="panel-body task-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (!busy && goal.trim() && provider)
            void onCreate({ goal: goal.trim(), constraints: constraints.trim(), provider });
        }}
      >
        <div className="field">
          <label htmlFor="task-goal">Goal</label>
          <textarea
            id="task-goal"
            value={goal}
            maxLength={16000}
            rows={5}
            disabled={busy}
            placeholder="For example: find why importing a large document stalls and make it reliable."
            onChange={(event) => setGoal(event.target.value)}
            required
          />
        </div>
        <div className="field">
          <label htmlFor="task-constraints">
            Constraints and context <span className="optional-label">optional</span>
          </label>
          <textarea
            id="task-constraints"
            value={constraints}
            maxLength={8000}
            rows={3}
            disabled={busy}
            placeholder="Relevant files, limits, decisions already made, or how you will judge the result."
            onChange={(event) => setConstraints(event.target.value)}
          />
        </div>
        <div className="field task-provider-field">
          <label htmlFor="task-provider">Provider</label>
          <select
            id="task-provider"
            value={provider}
            disabled={busy}
            onChange={(event) => setProvider(event.target.value)}
          >
            {studio.snapshot.registries.providers.map((item) => (
              <option key={item.name} value={item.name}>
                {item.name}
                {item.available ? "" : " · unavailable"}
              </option>
            ))}
          </select>
        </div>
        <div className="task-form-footer">
          <p className="form-note">Save the goal, then choose an agent call budget to begin.</p>
          <div className="inline-actions">
            {onCancel ? (
              <button type="button" className="button ghost" disabled={busy} onClick={onCancel}>
                Cancel
              </button>
            ) : null}
            <button
              type="submit"
              className="button primary"
              disabled={busy || !goal.trim() || !provider}
            >
              {busy ? "Saving…" : "Save task"}
              <Icon name="arrow" />
            </button>
          </div>
        </div>
      </form>
    </article>
  );
}

function TaskDetail({
  task,
  operating,
  blocked,
  anotherRunning,
  onContinue,
  onPause,
}: {
  readonly task: TaskDocument;
  readonly operating: boolean;
  readonly blocked: boolean;
  readonly anotherRunning: boolean;
  readonly onContinue: (maxCalls: number, note?: string) => Promise<boolean>;
  readonly onPause: () => void;
}) {
  const [note, setNote] = useState("");
  const [budget, setBudget] = useState(4);
  const [reconciled, setReconciled] = useState(false);
  const [now, setNow] = useState(Date.now);
  const latest = [...task.rounds]
    .reverse()
    .find((round) => round.role === "lead" && round.report)?.report;
  const activeRounds = task.rounds.filter((round) => round.outcome === "running");
  const running = task.status === "running";
  const unknown = task.status === "outcome_unknown";
  const needsInput = task.status === "needs_input";
  const followUp = task.status === "completed";
  const locked = operating || blocked || anotherRunning;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);
  const elapsed = task.rounds.reduce(
    (sum, round) =>
      sum +
      (round.elapsedMs ??
        (round.outcome === "running" ? Math.max(0, now - Date.parse(round.startedAt)) : 0)),
    0,
  );
  return (
    <>
      <article className="panel task-brief">
        <div className="task-brief-heading">
          <span
            className={`pill ${task.status === "completed" ? "success" : running ? "running" : unknown || needsInput ? "warning" : ""}`}
          >
            {statusLabels[task.status]}
          </span>
          <span className="task-meta">
            {task.provider} · {task.calls} agent {task.calls === 1 ? "call" : "calls"} ·{" "}
            {duration(elapsed)} call time
          </span>
        </div>
        <h2>{task.goal}</h2>
        {task.constraints ? <p className="task-constraints">{task.constraints}</p> : null}
        {task.message ? (
          <p className="task-message" role="status">
            {task.message}
          </p>
        ) : null}
        {activeRounds.length ? (
          <div className="task-current" aria-label="Current work">
            <h3>{activeRounds.length > 1 ? "Investigating in parallel" : "Current focus"}</h3>
            <ul>
              {activeRounds.map((round) => (
                <li key={round.id}>{round.focus}</li>
              ))}
            </ul>
          </div>
        ) : latest ? (
          <div className="task-current">
            <h3>Latest focus</h3>
            <p>{latest.focus}</p>
          </div>
        ) : (
          <p className="task-intro">
            The first agent call will inspect the problem and choose a useful next action.
          </p>
        )}
      </article>

      <article className="panel task-next">
        <PanelHeader
          title={
            running ? "Work in progress" : task.status === "completed" ? "Follow up" : "Next move"
          }
          subtitle={
            running
              ? "Progress is saved as calls finish."
              : "Give the next attempt a bounded budget."
          }
        />
        <div className="panel-body">
          {running ? (
            <div className="task-form-footer">
              <p className="form-note">
                {task.pauseRequested
                  ? "Pause requested. Waiting for the current calls to finish."
                  : "Pausing lets the current calls finish and saves their results."}
              </p>
              <button
                type="button"
                className="button"
                disabled={locked || task.pauseRequested}
                onClick={onPause}
              >
                {task.pauseRequested ? "Pause requested" : "Pause after current calls"}
              </button>
            </div>
          ) : (
            <form
              className="task-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (
                  locked ||
                  ((needsInput || followUp) && !note.trim()) ||
                  (unknown && (!reconciled || !note.trim()))
                )
                  return;
                const answer = unknown
                  ? `I checked the external outcome. ${note.trim()}`
                  : note.trim();
                void onContinue(budget, answer || undefined).then((accepted) => {
                  if (accepted) {
                    setNote("");
                    setReconciled(false);
                  }
                });
              }}
            >
              {needsInput && latest?.question ? (
                <p className="task-question">{latest.question}</p>
              ) : null}
              {unknown ? (
                <div className="task-unknown">
                  <h3>Check what happened before continuing</h3>
                  <p>
                    The previous call may have changed files or an external system. Inspect that
                    state and describe what is safe to do next.
                  </p>
                </div>
              ) : null}
              <div className="field">
                <label htmlFor="task-note">
                  {unknown
                    ? "What did you verify?"
                    : followUp
                      ? "Follow-up direction"
                      : needsInput
                        ? "Your answer"
                        : "Direction for the next attempt (optional)"}
                </label>
                <textarea
                  id="task-note"
                  rows={3}
                  value={note}
                  maxLength={unknown ? 7950 : 8000}
                  disabled={locked}
                  required={unknown || needsInput || followUp}
                  onChange={(event) => setNote(event.target.value)}
                />
              </div>
              {unknown ? (
                <label className="task-reconcile">
                  <input
                    type="checkbox"
                    checked={reconciled}
                    disabled={locked}
                    onChange={(event) => setReconciled(event.target.checked)}
                  />
                  I have checked the external state and described how to continue.
                </label>
              ) : null}
              <div className="task-form-footer">
                <div className="field task-budget-field">
                  <label htmlFor="task-budget">Agent call budget for this attempt</label>
                  <select
                    id="task-budget"
                    value={budget}
                    disabled={locked}
                    onChange={(event) => setBudget(Number(event.target.value))}
                  >
                    {[1, 2, 4, 8, 12, 20].map((value) => (
                      <option value={value} key={value}>
                        {value} {value === 1 ? "call" : "calls"}
                      </option>
                    ))}
                  </select>
                  <small>
                    Each call runs an agent, including its tools. Parallel investigations each
                    count.
                  </small>
                </div>
                <button
                  type="submit"
                  className="button primary"
                  disabled={
                    locked ||
                    ((needsInput || followUp) && !note.trim()) ||
                    (unknown && (!reconciled || !note.trim()))
                  }
                >
                  <Icon name="play" />
                  {operating
                    ? "Starting…"
                    : task.status === "ready"
                      ? "Begin task"
                      : task.status === "completed"
                        ? "Start follow-up"
                        : "Continue task"}
                </button>
              </div>
              {anotherRunning ? (
                <p className="form-note">
                  Another task is working. Pause it before starting this one.
                </p>
              ) : blocked ? (
                <p className="form-note">An action is in progress. Continue after it finishes.</p>
              ) : null}
            </form>
          )}
        </div>
      </article>

      {latest ? (
        <article className="panel task-report">
          <PanelHeader title="Where the work stands" subtitle="Latest report from the agent" />
          <div className="panel-body">
            <ReportContent report={latest} />
          </div>
        </article>
      ) : null}

      {task.rounds.length ? (
        <article className="panel task-history">
          <PanelHeader
            title="Work history"
            subtitle={`${task.rounds.length} agent ${task.rounds.length === 1 ? "call" : "calls"} · most recent first`}
          />
          <div className="panel-body">
            {[...task.rounds].reverse().map((round) => (
              <RoundRecord key={round.id} round={round} />
            ))}
          </div>
        </article>
      ) : null}
      {task.notes.length ? (
        <details className="panel task-notes">
          <summary>Your directions · {task.notes.length}</summary>
          <div className="panel-body">
            {[...task.notes].reverse().map((item, index) => (
              <div key={`${item.at}-${index}`}>
                <small>{dateTime(item.at)}</small>
                <p>{item.text}</p>
              </div>
            ))}
          </div>
        </details>
      ) : null}
    </>
  );
}

function ReportContent({ report }: { readonly report: TaskReport }) {
  return (
    <div className="task-report-content">
      <span className="task-action-label">{actionLabels[report.action]}</span>
      <p className="task-summary">{report.summary}</p>
      {report.findings.length ? (
        <section>
          <h3>Findings</h3>
          <ul className="task-findings">
            {report.findings.map((finding, index) => (
              <li key={index}>
                <p>{finding.statement}</p>
                <small>
                  {finding.source
                    ? `Source: ${finding.source}`
                    : "No source supplied · agent inference"}
                </small>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {report.unknowns.length ? (
        <section>
          <h3>Still unknown</h3>
          <ul>
            {report.unknowns.map((item, index) => (
              <li key={index}>{item}</li>
            ))}
          </ul>
        </section>
      ) : null}
      <section>
        <h3>Decision</h3>
        <p>{report.decision}</p>
      </section>
      {report.artifacts.length ? (
        <section>
          <h3>Artifacts</h3>
          <ul className="task-artifacts">
            {report.artifacts.map((artifact, index) => (
              <li key={index}>{artifact}</li>
            ))}
          </ul>
        </section>
      ) : null}
      {report.checks.length ? (
        <section>
          <h3>Reported checks</h3>
          <p className="task-evidence-note">
            These results were reported by the agent. Motivo has not independently verified them.
          </p>
          <ul className="task-checks">
            {report.checks.map((check, index) => (
              <li key={index}>
                <div>
                  <strong>{check.name}</strong>
                  <span
                    className={`pill ${check.result === "passed" ? "success" : check.result === "failed" ? "error" : "warning"}`}
                  >
                    reported {check.result}
                  </span>
                </div>
                <p>{check.detail}</p>
                {check.source ? <small>Source: {check.source}</small> : null}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {report.next ? (
        <section>
          <h3>Proposed next move</h3>
          <p>{report.next}</p>
        </section>
      ) : null}
    </div>
  );
}

function RoundRecord({ round }: { readonly round: TaskRound }) {
  return (
    <details className="task-round">
      <summary>
        <span className="task-round-title">
          <strong>
            {round.report
              ? actionLabels[round.report.action]
              : round.role === "investigator"
                ? "Investigation"
                : "Agent call"}
            {round.role === "investigator" ? " · parallel inquiry" : ""}
          </strong>
          <span>{round.focus}</span>
        </span>
        <span className="task-round-time">
          {dateTime(round.startedAt)}
          <small>
            {round.outcome.replaceAll("_", " ")}
            {round.elapsedMs !== undefined ? ` · ${duration(round.elapsedMs)}` : ""}
          </small>
        </span>
      </summary>
      {round.report ? (
        <ReportContent report={round.report} />
      ) : (
        <p>{round.error ?? "Waiting for this call to finish."}</p>
      )}
      {round.report && round.error ? <p className="task-message">{round.error}</p> : null}
      {round.rawOutput !== undefined ? (
        <details className="task-raw-response">
          <summary>Raw agent response{round.rawOutputTruncated ? " (truncated)" : ""}</summary>
          <p>Unparsed response · diagnostic text.</p>
          <pre>{round.rawOutput}</pre>
        </details>
      ) : null}
      {round.runId ? <p className="task-run-reference">Run evidence: {round.runId}</p> : null}
    </details>
  );
}

function duration(milliseconds: number): string {
  const seconds = Math.round(milliseconds / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return minutes < 60
    ? `${minutes}m ${seconds % 60}s`
    : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

function dateTime(value: string): string {
  return new Date(value).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
