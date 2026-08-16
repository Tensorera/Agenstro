import { useEffect, useRef } from "react";
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

  return (
    <section className="action-drawer" aria-label="Current action output">
      <header className="action-head">
        <span className={`status-dot ${action.status}`} aria-hidden="true" />
        <div className="action-title">
          <strong>{actionLabel(action.kind)}</strong>
          <span>
            {action.status} · started {formatTime(action.startedAtUnixMs)}
            {action.exitCode !== undefined ? ` · exit ${action.exitCode ?? "unknown"}` : ""}
          </span>
        </div>
        <div className="segmented" aria-label="Output stream">
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
        {running ? (
          <button
            type="button"
            className="button compact danger"
            onClick={onCancel}
            disabled={action.status === "cancelling"}
          >
            <Icon name="stop" /> {action.status === "cancelling" ? "Cancelling…" : "Cancel"}
          </button>
        ) : (
          <button
            type="button"
            className="icon-button"
            aria-label="Close action output"
            onClick={onClose}
          >
            <Icon name="close" />
          </button>
        )}
      </header>
      <div className="action-body">
        {action.message ? (
          <div className={`action-message ${action.status}`} role="status">
            {action.message}
          </div>
        ) : null}
        <pre ref={outputRef} className={`action-output ${stream}`} aria-live="polite">
          {output || <span className="placeholder">Waiting for {stream}…</span>}
        </pre>
      </div>
    </section>
  );
}
