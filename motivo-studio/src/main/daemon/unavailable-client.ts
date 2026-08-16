import type {
  FileDocument,
  FilePage,
  Recovery,
  RecoveryPage,
  Run,
  SchedulePage,
  StudioSnapshot,
} from "../../shared/contracts";
import type { DaemonClient, OpenedWorkspace, RunObserver, RunWatch } from "./daemon-client";
import { unavailable } from "../errors";
import { RELEASE_VERSION } from "../../shared/release";

const SNAPSHOT: StudioSnapshot = {
  state: "degraded",
  version: RELEASE_VERSION,
  services: [
    { service: "agentrod", state: "unavailable", detail: "Bundled daemon unavailable" },
    { service: "tactusd", state: "unavailable", detail: "Bundled daemon unavailable" },
    { service: "segnod", state: "unavailable", detail: "Service discovery unavailable" },
  ],
};

export class UnavailableDaemonClient implements DaemonClient {
  snapshot(): StudioSnapshot {
    return SNAPSHOT;
  }

  openWorkspace(): Promise<OpenedWorkspace> {
    return Promise.reject(unavailable("tactusd is not connected."));
  }

  terminalCwd(): string {
    throw unavailable("The workspace is not connected.");
  }

  listFiles(): Promise<FilePage> {
    return Promise.reject(unavailable("tactusd is not connected."));
  }

  readFile(): Promise<FileDocument> {
    return Promise.reject(unavailable("tactusd is not connected."));
  }

  saveFile(): Promise<FileDocument> {
    return Promise.reject(unavailable("tactusd is not connected."));
  }

  startRun(): Promise<Run> {
    return Promise.reject(unavailable("agentrod is not connected."));
  }

  getRun(): Promise<Run> {
    return Promise.reject(unavailable("agentrod is not connected."));
  }

  cancelRun(): Promise<Run> {
    return Promise.reject(unavailable("agentrod is not connected."));
  }

  watchRun(_runId: string, _afterSequence: string, observer: RunObserver): RunWatch {
    queueMicrotask(() => observer.onError(unavailable("agentrod is not connected.").detail));
    return { cancel: () => undefined };
  }

  listSchedules(): Promise<SchedulePage> {
    return Promise.reject(unavailable("segnod is not connected."));
  }

  listRecoveries(): Promise<RecoveryPage> {
    return Promise.reject(unavailable("tactusd is not connected."));
  }

  recover(): Promise<Recovery> {
    return Promise.reject(unavailable("tactusd is not connected."));
  }

  close(): void {}
}
