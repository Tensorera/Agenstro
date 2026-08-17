import type { ActionState, StudioPresentation } from "../shared/contracts";

export type NavigationView = "overview" | "workflow" | "plugins" | "runs";
export type ActionStatus = "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
export type OutputStream = "stdout" | "stderr";

export interface ActionPresentation extends StudioPresentation {
  readonly sequence: string;
}

export interface ActiveAction extends ActionState {
  readonly status: ActionStatus;
  readonly stdout: string;
  readonly stderr: string;
  readonly stdoutChunks: number;
  readonly stderrChunks: number;
  readonly presentations: readonly ActionPresentation[];
  readonly exitCode?: number | null;
  readonly message?: string | undefined;
}

export const EVENT_PAGE_SIZE = 100;
export const MAX_RENDERED_OUTPUT = 240_000;

export function isActionBusy(action: ActiveAction | null): boolean {
  return action?.status === "running" || action?.status === "cancelling";
}
