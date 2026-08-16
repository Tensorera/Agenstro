import type { RunState, TaskState } from "../types/segnoFlow";
import { sentenceCase } from "../lib/format";

export function StatusPill({ status, compact = false }: { status: TaskState | RunState; compact?: boolean }) {
  return (
    <span className={`status-pill status-${status} ${compact ? "compact" : ""}`}>
      <span className="status-dot" aria-hidden="true" />
      {sentenceCase(status)}
    </span>
  );
}
