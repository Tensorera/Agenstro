import {
  credentials,
  Metadata,
  type CallOptions,
  type Client,
  type ClientUnaryCall,
  type ServiceError,
} from "@grpc/grpc-js";
import {
  RunServiceClient,
  RunState,
  type Run as WireRun,
  type RunEvent as WireRunEvent,
} from "../../generated/agentro/execution/v1/run_service";
import {
  ScheduleServiceClient,
  ScheduleState,
} from "../../generated/agentro/schedule/v1/schedule_service";
import { Product, SystemServiceClient } from "../../generated/agentro/system/v1/system_service";
import {
  EntryKind,
  RecoveryState,
  WorkspaceServiceClient,
  type FileDocument as WireFileDocument,
  type RecoveryRecord as WireRecovery,
} from "../../generated/agentro/workspace/v1/workspace_service";
import {
  fileDocumentSchema,
  filePageSchema,
  LIMITS,
  recoveryPageSchema,
  recoverySchema,
  runEventSchema,
  runSchema,
  schedulePageSchema,
  studioSnapshotSchema,
  workspaceSchema,
  type FileDocument,
  type FilePage,
  type Recovery,
  type RecoveryPage,
  type Run,
  type RunEvent,
  type SchedulePage,
  type StudioSnapshot,
  type WorkspaceId,
} from "../../shared/contracts";
import type { DaemonClient, OpenedWorkspace, RunObserver, RunWatch } from "./daemon-client";
import { grpcFault, StudioFault, unavailable } from "../errors";
import { RELEASE_VERSION } from "../../shared/release";

export type DaemonProduct = "clef-sdk" | "tactus-runtime" | "segno-flow";

export interface DaemonConnection {
  readonly endpoint: string;
  readonly instanceId: string;
  readonly token: Buffer;
  readonly product: DaemonProduct;
}

export interface DaemonConnections {
  readonly agentrod: DaemonConnection;
  readonly tactusd: DaemonConnection;
  readonly segnod?: DaemonConnection;
}

type UnaryMethod<Request, Response> = (
  request: Request,
  metadata: Metadata,
  options: Partial<CallOptions>,
  callback: (error: ServiceError | null, response: Response) => void,
) => ClientUnaryCall;

const utf8Decoder = new TextDecoder("utf-8", { fatal: true });

export class GrpcDaemonClient implements DaemonClient {
  private readonly workspaceClient: WorkspaceServiceClient;
  private readonly runClient: RunServiceClient;
  private readonly scheduleClient: ScheduleServiceClient | undefined;
  private readonly clients: Client[];
  private readonly workspaceRoots = new Map<WorkspaceId, string>();
  private readonly snapshotValue: StudioSnapshot;

  private constructor(private readonly connections: DaemonConnections) {
    const channelOptions = {
      "grpc.max_receive_message_length": LIMITS.fileBytes + 65_536,
      "grpc.max_send_message_length": LIMITS.fileBytes + 65_536,
    };
    this.workspaceClient = new WorkspaceServiceClient(
      connections.tactusd.endpoint,
      credentials.createInsecure(),
      channelOptions,
    );
    this.runClient = new RunServiceClient(
      connections.agentrod.endpoint,
      credentials.createInsecure(),
      channelOptions,
    );
    this.scheduleClient = connections.segnod
      ? new ScheduleServiceClient(
          connections.segnod.endpoint,
          credentials.createInsecure(),
          channelOptions,
        )
      : undefined;
    this.clients = [this.workspaceClient, this.runClient];
    if (this.scheduleClient) {
      this.clients.push(this.scheduleClient);
    }
    this.snapshotValue = studioSnapshotSchema.parse({
      state: connections.segnod ? "ready" : "degraded",
      version: RELEASE_VERSION,
      services: [
        { service: "agentrod", state: "ready", instanceId: connections.agentrod.instanceId },
        { service: "tactusd", state: "ready", instanceId: connections.tactusd.instanceId },
        connections.segnod
          ? { service: "segnod", state: "ready", instanceId: connections.segnod.instanceId }
          : { service: "segnod", state: "unavailable", detail: "Service discovery unavailable" },
      ],
    });
  }

  static async connect(connections: DaemonConnections): Promise<GrpcDaemonClient> {
    const client = new GrpcDaemonClient(connections);
    try {
      await Promise.all([
        client.probe(connections.agentrod),
        client.probe(connections.tactusd),
        connections.segnod ? client.probe(connections.segnod) : Promise.resolve(),
      ]);
      return client;
    } catch (error) {
      client.close();
      throw error;
    }
  }

  snapshot(): StudioSnapshot {
    return this.snapshotValue;
  }

  async openWorkspace(nativePath: string, requestId: string): Promise<OpenedWorkspace> {
    const response = await this.unary(
      this.workspaceClient.openWorkspace.bind(this.workspaceClient),
      { requestId, authorizedNativePath: nativePath },
      this.connections.tactusd,
    );
    const workspace = workspaceSchema.parse({
      id: response.workspaceId,
      name: response.displayName,
      revision: response.revision,
      rootEntryId: response.rootEntryId,
    });
    this.workspaceRoots.set(workspace.id, nativePath);
    return { workspace, terminalCwd: nativePath };
  }

  terminalCwd(workspaceId: WorkspaceId): string {
    const root = this.workspaceRoots.get(workspaceId);
    if (!root) {
      throw unavailable("The workspace terminal context is no longer available.");
    }
    return root;
  }

  async listFiles(input: Parameters<DaemonClient["listFiles"]>[0]): Promise<FilePage> {
    const response = await this.unary(
      this.workspaceClient.listEntries.bind(this.workspaceClient),
      {
        workspaceId: input.workspaceId,
        pageSize: input.pageSize,
        parentEntryId: input.parentId,
        cursor: input.cursor,
      },
      this.connections.tactusd,
    );
    return filePageSchema.parse({
      workspaceId: input.workspaceId,
      parentId: input.parentId,
      entries: response.entries.map((entry) => ({
        id: entry.entryId,
        parentId: entry.parentEntryId,
        name: entry.name,
        kind: mapEntryKind(entry.kind),
        sizeBytes: safeNumber(entry.sizeBytes),
        language: entry.language,
        revision: entry.revision,
        readOnly: entry.readOnly,
      })),
      nextCursor: response.nextCursor,
    });
  }

  async readFile(input: Parameters<DaemonClient["readFile"]>[0]): Promise<FileDocument> {
    const response = await this.unary(
      this.workspaceClient.readFile.bind(this.workspaceClient),
      { workspaceId: input.workspaceId, entryId: input.entryId, maxBytes: LIMITS.fileBytes },
      this.connections.tactusd,
    );
    return mapFile(response);
  }

  async saveFile(input: Parameters<DaemonClient["saveFile"]>[0]): Promise<FileDocument> {
    const response = await this.unary(
      this.workspaceClient.saveFile.bind(this.workspaceClient),
      {
        requestId: input.requestId,
        workspaceId: input.workspaceId,
        entryId: input.entryId,
        content: Buffer.from(input.content, "utf8"),
        expectedRevision: input.expectedRevision,
      },
      this.connections.tactusd,
    );
    return mapFile(response);
  }

  async startRun(input: Parameters<DaemonClient["startRun"]>[0]): Promise<Run> {
    const response = await this.unary(
      this.runClient.startRun.bind(this.runClient),
      input,
      this.connections.agentrod,
    );
    return mapRun(response);
  }

  async getRun(input: Parameters<DaemonClient["getRun"]>[0]): Promise<Run> {
    const response = await this.unary(
      this.runClient.getRun.bind(this.runClient),
      input,
      this.connections.agentrod,
    );
    return mapRun(response);
  }

  async cancelRun(input: Parameters<DaemonClient["cancelRun"]>[0]): Promise<Run> {
    const response = await this.unary(
      this.runClient.cancelRun.bind(this.runClient),
      input,
      this.connections.agentrod,
    );
    return mapRun(response);
  }

  watchRun(runId: string, afterSequence: string, observer: RunObserver): RunWatch {
    const stream = this.runClient.watchRun(
      { runId, afterSequence: BigInt(afterSequence) },
      metadata(this.connections.agentrod.token),
      { deadline: Date.now() + 60 * 60 * 1_000 },
    );
    stream.on("data", (event: WireRunEvent) => {
      try {
        observer.onEvent(mapRunEvent(event));
      } catch (error) {
        stream.cancel();
        observer.onError(
          error instanceof StudioFault
            ? error.detail
            : {
                code: "RPC_PROTOCOL_ERROR",
                category: "internal",
                retryable: false,
                message: "The daemon returned an invalid run event.",
              },
        );
      }
    });
    stream.on("error", (error: ServiceError) => observer.onError(grpcFault(error).detail));
    stream.on("end", () => observer.onEnd());
    return { cancel: () => stream.cancel() };
  }

  async listSchedules(input: Parameters<DaemonClient["listSchedules"]>[0]): Promise<SchedulePage> {
    if (!this.scheduleClient || !this.connections.segnod) {
      throw unavailable("segnod is not connected.");
    }
    const response = await this.unary(
      this.scheduleClient.listSchedules.bind(this.scheduleClient),
      input,
      this.connections.segnod,
    );
    return schedulePageSchema.parse({
      schedules: response.schedules.map((schedule) => ({
        id: schedule.scheduleId,
        taskId: schedule.taskId,
        label: schedule.label,
        cron: schedule.cron,
        timezone: schedule.timezone,
        state: mapScheduleState(schedule.state),
        nextFireAt: schedule.nextFireAt,
        lastRunId: schedule.lastRunId,
      })),
      nextCursor: response.nextCursor,
    });
  }

  async listRecoveries(
    input: Parameters<DaemonClient["listRecoveries"]>[0],
  ): Promise<RecoveryPage> {
    const response = await this.unary(
      this.workspaceClient.listRecoveries.bind(this.workspaceClient),
      input,
      this.connections.tactusd,
    );
    return recoveryPageSchema.parse({
      records: response.recoveries.map(mapRecovery),
      nextCursor: response.nextCursor,
    });
  }

  async recover(input: Parameters<DaemonClient["recover"]>[0]): Promise<Recovery> {
    const response = await this.unary(
      this.workspaceClient.recoverWorkspace.bind(this.workspaceClient),
      input,
      this.connections.tactusd,
    );
    return mapRecovery(response);
  }

  close(): void {
    this.clients.forEach((client) => client.close());
    this.connections.agentrod.token.fill(0);
    this.connections.tactusd.token.fill(0);
    this.connections.segnod?.token.fill(0);
    this.workspaceRoots.clear();
  }

  private async probe(connection: DaemonConnection): Promise<void> {
    const client = new SystemServiceClient(connection.endpoint, credentials.createInsecure());
    try {
      const response = await this.unary(client.getServerInfo.bind(client), {}, connection, 3_000);
      const info = response.serverInfo;
      const minimum = info?.apiVersions?.minimum;
      const maximum = info?.apiVersions?.maximum;
      const descriptor = info?.protocolDescriptor;
      if (
        !info ||
        info.instanceId !== connection.instanceId ||
        productName(info.product) !== connection.product ||
        minimum?.major !== 1 ||
        maximum?.major !== 1 ||
        minimum.minor > 0 ||
        maximum.minor < 0 ||
        info.releaseVersion.length === 0 ||
        descriptor?.algorithm !== "sha256" ||
        descriptor.value.byteLength !== 32 ||
        info.capabilities.length > 128
      ) {
        throw unavailable("The daemon protocol or instance identity is incompatible.");
      }
    } finally {
      client.close();
    }
  }

  private unary<Request, Response>(
    method: UnaryMethod<Request, Response>,
    request: Request,
    connection: DaemonConnection,
    deadlineMs = 10_000,
  ): Promise<Response> {
    return new Promise((resolve, reject) => {
      method(
        request,
        metadata(connection.token),
        { deadline: Date.now() + deadlineMs },
        (error, response) => {
          if (error) {
            reject(grpcFault(error));
          } else {
            resolve(response);
          }
        },
      );
    });
  }
}

function productName(product: Product): DaemonProduct | undefined {
  switch (product) {
    case Product.PRODUCT_CLEF_SDK:
      return "clef-sdk";
    case Product.PRODUCT_TACTUS_RUNTIME:
      return "tactus-runtime";
    case Product.PRODUCT_SEGNO_FLOW:
      return "segno-flow";
    case Product.PRODUCT_MOTIVO_STUDIO:
    case Product.PRODUCT_UNSPECIFIED:
    case Product.UNRECOGNIZED:
      return undefined;
  }
}

function metadata(token: Buffer): Metadata {
  const value = new Metadata();
  value.set("authorization", `Bearer ${token.toString("base64url")}`);
  return value;
}

function mapRunState(state: RunState): Run["state"] {
  switch (state) {
    case RunState.RUN_STATE_QUEUED:
      return "queued";
    case RunState.RUN_STATE_RUNNING:
      return "running";
    case RunState.RUN_STATE_RECOVERING:
      return "recovering";
    case RunState.RUN_STATE_SUCCEEDED:
      return "succeeded";
    case RunState.RUN_STATE_FAILED:
      return "failed";
    case RunState.RUN_STATE_CANCELLED:
      return "cancelled";
    case RunState.RUN_STATE_UNSPECIFIED:
    case RunState.UNRECOGNIZED:
      throw new StudioFault({
        code: "RPC_UNKNOWN_RUN_STATE",
        category: "internal",
        retryable: false,
        message: "The daemon returned an unsupported run state.",
      });
  }
}

function mapRun(value: WireRun): Run {
  return runSchema.parse({
    id: value.runId,
    workspaceId: value.workspaceId,
    state: mapRunState(value.state),
    lastSequence: value.lastSequence.toString(),
    updatedAt: value.updatedAt,
    detail: value.statusDetail,
  });
}

function mapRunEvent(value: WireRunEvent): RunEvent {
  if (!value.occurredAt || !value.body) {
    throw unavailable("The daemon returned an incomplete run event.");
  }
  let body: RunEvent["body"];
  switch (value.body.$case) {
    case "started":
      body = { kind: "started", label: value.body.value.label };
      break;
    case "stageChanged":
      body = {
        kind: "stage",
        stageId: value.body.value.stageId,
        label: value.body.value.label,
        state: value.body.value.state,
      };
      break;
    case "output":
      body = {
        kind: "output",
        stream:
          value.body.value.stream === "stdout" || value.body.value.stream === "stderr"
            ? value.body.value.stream
            : "system",
        data: utf8Decoder.decode(value.body.value.data),
        truncated: value.body.value.truncated,
      };
      break;
    case "diagnostic":
      body = {
        kind: "diagnostic",
        code: value.body.value.code,
        message: value.body.value.message,
      };
      break;
    case "finished":
      body = {
        kind: "finished",
        state: mapRunState(value.body.value.state),
        summary: value.body.value.summary,
      };
      break;
  }
  return runEventSchema.parse({
    runId: value.runId,
    sequence: value.sequence.toString(),
    occurredAt: value.occurredAt,
    body,
  });
}

function mapEntryKind(kind: EntryKind): "file" | "directory" {
  if (kind === EntryKind.ENTRY_KIND_FILE) return "file";
  if (kind === EntryKind.ENTRY_KIND_DIRECTORY) return "directory";
  throw unavailable("The daemon returned an unsupported file kind.");
}

function safeNumber(value: bigint): number {
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw unavailable("The daemon returned a file size outside the supported range.");
  }
  return Number(value);
}

function mapFile(value: WireFileDocument): FileDocument {
  let content = "";
  if (!value.binary) {
    try {
      content = utf8Decoder.decode(value.content);
    } catch {
      throw unavailable("The daemon returned non-UTF-8 editor content.");
    }
  }
  return fileDocumentSchema.parse({
    workspaceId: value.workspaceId,
    entryId: value.entryId,
    name: value.name,
    content,
    revision: value.revision,
    language: value.language,
    readOnly: value.readOnly,
    binary: value.binary,
    truncated: value.truncated,
  });
}

function mapScheduleState(state: ScheduleState) {
  switch (state) {
    case ScheduleState.SCHEDULE_STATE_DISABLED:
      return "disabled" as const;
    case ScheduleState.SCHEDULE_STATE_READY:
      return "ready" as const;
    case ScheduleState.SCHEDULE_STATE_DISPATCHING:
      return "dispatching" as const;
    case ScheduleState.SCHEDULE_STATE_RECOVERY_REQUIRED:
      return "recovery-required" as const;
    case ScheduleState.SCHEDULE_STATE_UNSPECIFIED:
    case ScheduleState.UNRECOGNIZED:
      throw unavailable("The daemon returned an unsupported schedule state.");
  }
}

function mapRecoveryState(state: RecoveryState): Recovery["state"] {
  switch (state) {
    case RecoveryState.RECOVERY_STATE_AVAILABLE:
      return "available";
    case RecoveryState.RECOVERY_STATE_APPLIED:
      return "applied";
    case RecoveryState.RECOVERY_STATE_CONFLICTED:
      return "conflicted";
    case RecoveryState.RECOVERY_STATE_EXPIRED:
      return "expired";
    case RecoveryState.RECOVERY_STATE_UNSPECIFIED:
    case RecoveryState.UNRECOGNIZED:
      throw unavailable("The daemon returned an unsupported recovery state.");
  }
}

function mapRecovery(value: WireRecovery): Recovery {
  return recoverySchema.parse({
    id: value.recoveryId,
    workspaceId: value.workspaceId,
    label: value.label,
    state: mapRecoveryState(value.state),
    createdAt: value.createdAt,
    changedFiles: value.changedFiles,
    detail: value.detail,
  });
}
