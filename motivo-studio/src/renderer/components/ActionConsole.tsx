import { useEffect, useRef } from "react";
import type { PresentationCategory } from "../../shared/contracts";
import { actionLabel, formatTime } from "../format";
import { isActionBusy, type ActiveAction, type OutputStream } from "../model";
import { Icon } from "./Icon";

interface ActionConsoleProps {
  readonly action: ActiveAction;
  readonly stream: OutputStream;
  readonly onStream: (stream: OutputStream) => void;
  readonly onCancel: () => void;
  readonly onClose: () => void;
}

export function ActionConsole({ action, stream, onStream, onCancel, onClose }: ActionConsoleProps) {
  const output = action[stream];
  const running = isActionBusy(action);
  const outputRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const element = outputRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [output, stream]);

  const hasCanonicalStart = action.presentations.some(
    (presentation) => presentation.category === "state",
  );
  const terminal = terminalPresentation(action);

  return (
    <section className="action-drawer" aria-label="Current action output">
      <header className="action-head">
        <span className={`status-dot ${action.status}`} aria-hidden="true" />
        <div className="action-title">
          <strong>{actionLabel(action.kind)}</strong>
          <span>
            {action.status} · started {formatTime(action.startedAtUnixMs)}
          </span>
        </div>
        {running ? (
          <button
            type="button"
            className="button compact danger action-control"
            onClick={onCancel}
            disabled={action.status === "cancelling"}
          >
            <Icon name="stop" /> {action.status === "cancelling" ? "Cancelling…" : "Cancel"}
          </button>
        ) : (
          <button
            type="button"
            className="icon-button action-control"
            aria-label="Close action output"
            onClick={onClose}
          >
            <Icon name="close" />
          </button>
        )}
      </header>
      <div className="action-body">
        <div className="presentation-log" aria-live="polite">
          {!hasCanonicalStart ? (
            <PresentationLine category="state" message={`${actionLabel(action.kind)} started.`} />
          ) : null}
          {action.presentations.map((presentation) => (
            <PresentationLine
              key={presentation.sequence}
              category={presentation.category}
              message={presentation.message}
            />
          ))}
          {terminal ? (
            <PresentationLine category={terminal.category} message={terminal.message} />
          ) : null}
        </div>
        <details className="action-technical">
          <summary>
            Technical details · stdout {action.stdoutChunks} · stderr {action.stderrChunks}
            {action.exitCode !== undefined ? ` · exit ${action.exitCode ?? "unknown"}` : ""}
          </summary>
          <div className="segmented" aria-label="Raw output stream">
            <button
              type="button"
              className={stream === "stdout" ? "active" : ""}
              aria-pressed={stream === "stdout"}
              onClick={() => onStream("stdout")}
            >
              stdout <span className="stream-count">{action.stdoutChunks}</span>
            </button>
            <button
              type="button"
              className={stream === "stderr" ? "active" : ""}
              aria-pressed={stream === "stderr"}
              onClick={() => onStream("stderr")}
            >
              stderr <span className="stream-count">{action.stderrChunks}</span>
            </button>
          </div>
          {action.message ? <pre className="action-diagnostic">{action.message}</pre> : null}
          <pre ref={outputRef} className={`action-output ${stream}`}>
            {output || <span className="placeholder">No raw {stream} output.</span>}
          </pre>
        </details>
      </div>
    </section>
  );
}

function PresentationLine({
  category,
  message,
}: {
  readonly category: PresentationCategory;
  readonly message: string;
}) {
  return (
    <p className={`presentation-line ${category}`}>
      <span className="presentation-tag">[{category}]</span>
      <span>{message}</span>
    </p>
  );
}

function terminalPresentation(
  action: ActiveAction,
): { readonly category: PresentationCategory; readonly message: string } | null {
  const stateMessages = action.presentations.filter(
    (presentation) => presentation.category === "state",
  ).length;
  const hasCanonicalTerminal =
    stateMessages > 1 ||
    action.presentations.some((presentation) => presentation.category === "error");
  if (hasCanonicalTerminal) return null;

  switch (action.status) {
    case "running":
      return null;
    case "cancelling":
      return { category: "state", message: "Cancellation requested." };
    case "succeeded":
      return { category: "state", message: `${actionLabel(action.kind)} completed successfully.` };
    case "cancelled":
      return {
        category: "warning",
        message: `${actionLabel(action.kind)} was cancelled.`,
      };
    case "failed":
      return {
        category: "error",
        message: `${actionLabel(action.kind)} failed. Open technical details for diagnostics.`,
      };
  }
}
