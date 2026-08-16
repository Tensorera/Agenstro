import type {
  FileDocument,
  FilePage,
  Recovery,
  RecoveryPage,
  Run,
  RunEvent,
  SchedulePage,
  StudioError,
  StudioSnapshot,
  Workspace,
  WorkspaceId,
} from "../../shared/contracts";
import type {
  filePageInputSchema,
  fileReadInputSchema,
  fileSaveInputSchema,
  pageInputSchema,
  recoveryApplyInputSchema,
  recoveryPageInputSchema,
  runCancelInputSchema,
  runGetInputSchema,
  runStartInputSchema,
} from "../../shared/ipc";
import type { z } from "zod";

export interface OpenedWorkspace {
  readonly workspace: Workspace;
  readonly terminalCwd: string;
}

export interface RunObserver {
  onEvent(event: RunEvent): void;
  onError(error: StudioError): void;
  onEnd(): void;
}

export interface RunWatch {
  cancel(): void;
}

export interface DaemonClient {
  snapshot(): StudioSnapshot;
  openWorkspace(nativePath: string, requestId: string): Promise<OpenedWorkspace>;
  terminalCwd(workspaceId: WorkspaceId): string;
  listFiles(input: z.infer<typeof filePageInputSchema>): Promise<FilePage>;
  readFile(input: z.infer<typeof fileReadInputSchema>): Promise<FileDocument>;
  saveFile(input: z.infer<typeof fileSaveInputSchema>): Promise<FileDocument>;
  startRun(input: z.infer<typeof runStartInputSchema>): Promise<Run>;
  getRun(input: z.infer<typeof runGetInputSchema>): Promise<Run>;
  cancelRun(input: z.infer<typeof runCancelInputSchema>): Promise<Run>;
  watchRun(runId: string, afterSequence: string, observer: RunObserver): RunWatch;
  listSchedules(input: z.infer<typeof pageInputSchema>): Promise<SchedulePage>;
  listRecoveries(input: z.infer<typeof recoveryPageInputSchema>): Promise<RecoveryPage>;
  recover(input: z.infer<typeof recoveryApplyInputSchema>): Promise<Recovery>;
  close(): void;
}
