import { useMemo, useState } from "react";
import { utf8Bytes, type StudioView } from "../../shared/contracts";
import {
  SESSION_LIMITS,
  type SessionAnswerInput,
  type SessionBrief,
  type SessionView,
} from "../../shared/session-contracts";
import { formatDateTime } from "../format";
import { Icon } from "./Icon";
import { PanelHeader, ViewHeader } from "./Primitives";

interface SessionsViewProps {
  readonly studio: StudioView | null;
  readonly sessions: readonly SessionView[] | null;
  readonly session: SessionView | null;
  readonly selectedSessionId: string | null;
  readonly busy: boolean;
  readonly answering: boolean;
  readonly actionBusy: boolean;
  readonly onSelect: (sessionId: string) => void;
  readonly onReload: () => void;
  readonly onAnswer: (input: SessionAnswerChoice) => void;
}

type SessionAnswerChoice = Omit<SessionAnswerInput, "workspaceHandle">;

export function SessionsView({
  studio,
  sessions,
  session,
  selectedSessionId,
  busy,
  answering,
  actionBusy,
  onSelect,
  onReload,
  onAnswer,
}: SessionsViewProps) {
  return (
    <>
      <ViewHeader
        eyebrow="Human decisions"
        title="Sessions"
        description="Review what Tactus learned, compare the consequences, and return one typed choice. The workspace remains the source of truth."
        action={
          <button
            type="button"
            className="button compact ghost"
            disabled={!studio || busy || answering || actionBusy}
            onClick={onReload}
          >
            <Icon name="refresh" /> {busy ? "Refreshing…" : "Refresh sessions"}
          </button>
        }
      />
      <div className="content-width">
        {!studio ? (
          <SessionEmpty title="No workspace connected" detail="Open a Tactus workspace first." />
        ) : sessions === null && busy ? (
          <SessionEmpty title="Reading sessions" detail="Loading the bounded session projection…" />
        ) : sessions?.length === 0 ? (
          <SessionEmpty
            title="No sessions found"
            detail="No sessions are available in this staged boundary. Motivo currently has no planner or publish command."
          />
        ) : (
          <>
            <section className="session-toolbar" aria-label="Session selection">
              <label htmlFor="session-picker">Session</label>
              <select
                id="session-picker"
                value={selectedSessionId ?? ""}
                disabled={busy || answering || actionBusy}
                onChange={(event) => onSelect(event.target.value)}
              >
                {(sessions ?? []).map((item) => (
                  <option key={item.sessionId} value={item.sessionId}>
                    {item.label} · {stateLabel(item.state)}
                  </option>
                ))}
              </select>
              {session ? (
                <span className={`pill session-state ${session.state}`}>
                  {stateLabel(session.state)}
                </span>
              ) : null}
            </section>

            {busy && !session ? (
              <SessionEmpty
                title="Reading the selected session"
                detail="Validating its current turn…"
              />
            ) : session ? (
              <section className="sessions-layout" aria-label={`${session.label} session`}>
                <div className="session-decision">
                  <PanelHeader
                    title={session.label}
                    subtitle={`Turn ${session.turn} · updated ${formatDateTime(session.updatedUnixMs)}`}
                  />
                  {session.pending ? (
                    <DecisionPane
                      key={`${session.sessionId}:${session.turn}`}
                      session={session}
                      brief={session.pending}
                      disabled={busy || answering || actionBusy}
                      answering={answering}
                      onAnswer={onAnswer}
                    />
                  ) : (
                    <SessionStateBody session={session} />
                  )}
                </div>

                <aside className="session-side">
                  <RoadmapPane session={session} />
                  <AnsweredPane session={session} />
                </aside>
              </section>
            ) : null}
          </>
        )}
      </div>
    </>
  );
}

function DecisionPane({
  session,
  brief,
  disabled,
  answering,
  onAnswer,
}: {
  readonly session: SessionView;
  readonly brief: SessionBrief;
  readonly disabled: boolean;
  readonly answering: boolean;
  readonly onAnswer: (input: SessionAnswerChoice) => void;
}) {
  const [selectedOption, setSelectedOption] = useState("");
  const [note, setNote] = useState("");
  const coordinateKeys = useMemo(
    () =>
      [
        ...new Set(brief.question.options.flatMap((option) => Object.keys(option.coordinates))),
      ].sort((left, right) => left.localeCompare(right)),
    [brief.question.options],
  );
  const noteBytes = utf8Bytes(note);
  const defaultLabel = brief.question.options.find(
    (option) => option.id === brief.defaultOption,
  )?.label;

  return (
    <div className="session-decision-body">
      <section className="brief-section" aria-labelledby="session-findings-title">
        <p className="eyebrow" id="session-findings-title">
          What I found out
        </p>
        {brief.findings.length === 0 ? (
          <p className="session-muted">No new findings were reported for this turn.</p>
        ) : (
          <ol className="finding-list">
            {brief.findings.map((finding, index) => (
              <li key={`${index}:${finding.summary}`}>
                <p>{finding.summary}</p>
                <span className={`finding-source ${finding.source ? "sourced" : "unsourced"}`}>
                  {finding.source ? (
                    <>Source · {finding.source}</>
                  ) : (
                    <>
                      <Icon name="warning" /> No source — inference
                    </>
                  )}
                </span>
                {finding.detail ? (
                  <details className="finding-detail">
                    <summary>Finding detail</summary>
                    <p>{finding.detail}</p>
                  </details>
                ) : null}
              </li>
            ))}
          </ol>
        )}
      </section>

      <section className="brief-section question-section">
        <div className="question-heading">
          <div>
            <p className="eyebrow">What only you can decide</p>
            <h2>{brief.question.prompt}</h2>
          </div>
          <span className={`pill reversibility ${brief.question.reversibility}`}>
            {reversibilityLabel(brief.question.reversibility)}
          </span>
        </div>

        <fieldset className="option-fieldset" disabled={disabled}>
          <legend className="sr-only">Choose one answer</legend>
          <div className="option-grid">
            {brief.question.options.map((option) => {
              const stakes = brief.stakes.filter((stake) => stake.option === option.id);
              return (
                <label
                  className={`session-option ${selectedOption === option.id ? "selected" : ""}`}
                  key={option.id}
                >
                  <input
                    type="radio"
                    name={`${session.sessionId}:${brief.turn}`}
                    value={option.id}
                    checked={selectedOption === option.id}
                    onChange={() => setSelectedOption(option.id)}
                  />
                  <strong>{option.label}</strong>
                  {coordinateKeys.length > 0 ? (
                    <dl className="option-coordinates">
                      {coordinateKeys.map((key) => (
                        <div key={key}>
                          <dt>{key}</dt>
                          <dd>{option.coordinates[key] ?? "—"}</dd>
                        </div>
                      ))}
                    </dl>
                  ) : null}
                  {option.rationale ? <p className="option-rationale">{option.rationale}</p> : null}
                  {stakes.map((stake) => (
                    <p className="option-stake" key={`${stake.option}:${stake.effect}`}>
                      <span aria-hidden="true">→</span> {stake.effect}
                      <small
                        className={`stake-reversibility ${stake.reversibility}`}
                        title={`This consequence is ${reversibilityLabel(stake.reversibility)}.`}
                      >
                        {reversibilityLabel(stake.reversibility)}
                      </small>
                    </p>
                  ))}
                </label>
              );
            })}
          </div>
        </fieldset>

        <p className={`session-default ${brief.defaultOption ? "available" : "required"}`}>
          {brief.defaultOption
            ? `Unattended default: ${defaultLabel ?? brief.defaultOption}. Motivo will never apply it on a timer.`
            : "No default is available. This session requires an explicit answer."}
        </p>

        <label className="session-note">
          <span>Optional note</span>
          <textarea
            aria-label="Optional note"
            value={note}
            disabled={disabled}
            placeholder="Add context the listed choices do not capture."
            onChange={(event) => {
              if (utf8Bytes(event.target.value) <= SESSION_LIMITS.noteBytes) {
                setNote(event.target.value);
              }
            }}
          />
          <small>
            {noteBytes}/{SESSION_LIMITS.noteBytes} UTF-8 bytes
          </small>
        </label>

        <div className="session-answer-row">
          <span>The turn token prevents a stale window from answering a newer question.</span>
          <button
            type="button"
            className="button primary"
            disabled={disabled || !selectedOption}
            onClick={() =>
              onAnswer({
                sessionId: session.sessionId,
                turn: brief.turn,
                axis: brief.question.axis,
                option: selectedOption,
                ...(note ? { note } : {}),
              })
            }
          >
            <Icon name="check" /> {answering ? "Recording…" : "Record answer"}
          </button>
        </div>
      </section>
    </div>
  );
}

function RoadmapPane({ session }: { readonly session: SessionView }) {
  const answered = new Set(session.answered.map((item) => item.axis));
  const floor = session.pending?.remainingFloor ?? [];
  const surface = session.pending?.remainingSurface ?? [];
  const floorSet = new Set(floor);
  const current = session.pending?.question.axis;
  const must = [
    ...floor.filter((axis) => !answered.has(axis) || axis === current),
    ...(current && !floorSet.has(current) ? [current] : []),
  ];
  const may = surface.filter(
    (axis) => axis !== current && !answered.has(axis) && !floorSet.has(axis),
  );

  return (
    <article className="panel session-roadmap">
      <PanelHeader title="Roadmap" subtitle="The bounded decision space" />
      <div className="panel-body roadmap-body">
        <RoadmapBand
          title="Decided"
          items={session.answered.map((answer) => ({ axis: answer.axis, detail: answer.label }))}
          empty="Nothing decided yet."
          tone="decided"
        />
        <RoadmapBand
          title={`Must still decide (${String(must.length)})`}
          items={must.map((axis) => ({
            axis,
            ...(axis === current ? { detail: answered.has(axis) ? "revisiting" : "current" } : {}),
          }))}
          empty="No required axes remain."
          tone="must"
        />
        <RoadmapBand
          title={`May be asked (${String(may.length)})`}
          items={may.map((axis) => ({ axis }))}
          empty="No conditional axes are projected."
          tone="conditional"
        />
        {session.pending ? (
          <p className="roadmap-note">
            “May be asked” is an over-approximation and depends on answers above.
          </p>
        ) : null}
      </div>
    </article>
  );
}

function RoadmapBand({
  title,
  items,
  empty,
  tone,
}: {
  readonly title: string;
  readonly items: readonly { readonly axis: string; readonly detail?: string }[];
  readonly empty: string;
  readonly tone: "decided" | "must" | "conditional";
}) {
  return (
    <section className={`roadmap-band ${tone}`}>
      <h3>{title}</h3>
      {items.length === 0 ? (
        <p>{empty}</p>
      ) : (
        <ul>
          {items.map((item) => (
            <li key={item.axis}>
              <span className="roadmap-mark" aria-hidden="true">
                {tone === "decided" ? "✓" : tone === "must" ? "●" : "○"}
              </span>
              <strong>{item.axis}</strong>
              {item.detail ? <small>{item.detail}</small> : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function AnsweredPane({ session }: { readonly session: SessionView }) {
  return (
    <article className="panel session-answered">
      <PanelHeader title="Answered" subtitle="Current right-biased decisions" />
      {session.answered.length === 0 ? (
        <div className="empty-list">No answers have been recorded.</div>
      ) : (
        <ol className="answered-list">
          {[...session.answered].reverse().map((answer) => (
            <li key={answer.axis}>
              <div>
                <strong>{answer.axis}</strong>
                <span>{answer.label}</span>
              </div>
              <small>{formatDateTime(answer.answeredAtUnixMs)}</small>
              {answer.defaulted ? <span className="pill warning">defaulted</span> : null}
            </li>
          ))}
        </ol>
      )}
    </article>
  );
}

function SessionStateBody({ session }: { readonly session: SessionView }) {
  const copy = {
    planning: {
      title: "Planning the next turn",
      detail:
        "No next brief is available. This staged boundary currently supports session list, show, and answer only.",
    },
    delivered: {
      title: "Session delivered",
      detail: "The planner converged and delivered its artifact.",
    },
    abandoned: {
      title: "Session abandoned",
      detail: "This session is terminal and has no pending decision.",
    },
    awaiting_answer: {
      title: "Pending brief unavailable",
      detail: "The validated session invariant was not satisfied.",
    },
  }[session.state];
  return <SessionEmpty title={copy.title} detail={copy.detail} compact />;
}

function SessionEmpty({
  title,
  detail,
  compact = false,
}: {
  readonly title: string;
  readonly detail: string;
  readonly compact?: boolean;
}) {
  return (
    <section className={`session-empty ${compact ? "compact" : ""}`}>
      <Icon name="sessions" />
      <strong>{title}</strong>
      <p>{detail}</p>
    </section>
  );
}

function stateLabel(state: SessionView["state"]): string {
  switch (state) {
    case "planning":
      return "planning";
    case "awaiting_answer":
      return "awaiting answer";
    case "delivered":
      return "delivered";
    case "abandoned":
      return "abandoned";
  }
}

function reversibilityLabel(value: SessionBrief["question"]["reversibility"]): string {
  switch (value) {
    case "reversible":
      return "reversible";
    case "costly":
      return "costly to reverse";
    case "irreversible":
      return "irreversible";
  }
}
