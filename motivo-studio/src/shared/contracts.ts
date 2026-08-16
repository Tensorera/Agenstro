import { z } from "zod";
import type { StudioSurface } from "./surface";

export const LIMITS = {
  cursorCharacters: 512,
  diagnosticCharacters: 4_096,
  fileBytes: 1_048_576,
  filePage: 100,
  labelCharacters: 256,
  runBatchBytes: 262_144,
  runBatchEvents: 32,
  subscriptionsPerWindow: 8,
  terminalChunkBytes: 65_536,
  terminalInputBytes: 65_536,
} as const;

const textEncoder = new TextEncoder();

export function utf8Bytes(value: string): number {
  return textEncoder.encode(value).byteLength;
}

function boundedUtf8(maximumBytes: number) {
  return z.string().refine((value) => utf8Bytes(value) <= maximumBytes, {
    message: `Value exceeds the ${String(maximumBytes)} byte limit.`,
  });
}

const opaqueId = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[A-Za-z0-9][A-Za-z0-9._:-]*$/);
export const workspaceIdSchema = opaqueId.brand<"WorkspaceId">();
export const entryIdSchema = opaqueId.brand<"EntryId">();
export const runIdSchema = opaqueId.brand<"RunId">();
export const scheduleIdSchema = opaqueId.brand<"ScheduleId">();
export const recoveryIdSchema = opaqueId.brand<"RecoveryId">();
export const terminalIdSchema = opaqueId.brand<"TerminalId">();
export const subscriptionIdSchema = z.uuid().brand<"SubscriptionId">();
export const requestIdSchema = z.uuid().brand<"RequestId">();
export const sequenceSchema = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .brand<"Sequence">();
export const cursorSchema = z.string().min(1).max(LIMITS.cursorCharacters);
export const pageSizeSchema = z.number().int().min(1).max(LIMITS.filePage);
export const utcTimestampSchema = z.iso.datetime({ offset: true });

export type WorkspaceId = z.infer<typeof workspaceIdSchema>;
export type EntryId = z.infer<typeof entryIdSchema>;
export type RunId = z.infer<typeof runIdSchema>;
export type TerminalId = z.infer<typeof terminalIdSchema>;
export type SubscriptionId = z.infer<typeof subscriptionIdSchema>;
export type Sequence = z.infer<typeof sequenceSchema>;

export const studioErrorSchema = z
  .object({
    code: z
      .string()
      .min(1)
      .max(96)
      .regex(/^[A-Z][A-Z0-9_]*$/),
    category: z.enum(["validation", "connection", "conflict", "resource", "internal"]),
    retryable: z.boolean(),
    message: z.string().min(1).max(LIMITS.diagnosticCharacters),
    userAction: z.string().max(512).optional(),
    correlationId: opaqueId.optional(),
  })
  .strict();
export type StudioError = z.infer<typeof studioErrorSchema>;

export type IpcResult<T> =
  { readonly ok: true; readonly data: T } | { readonly ok: false; readonly error: StudioError };

export const ipcEnvelopeSchema = z.discriminatedUnion("ok", [
  z.object({ ok: z.literal(true), data: z.unknown() }).strict(),
  z.object({ ok: z.literal(false), error: studioErrorSchema }).strict(),
]);

export const serviceStatusSchema = z
  .object({
    service: z.enum(["agentrod", "tactusd", "segnod"]),
    state: z.enum(["ready", "starting", "unavailable"]),
    instanceId: opaqueId.optional(),
    detail: z.string().max(512).optional(),
  })
  .strict();

export const studioSnapshotSchema = z
  .object({
    state: z.enum(["ready", "degraded", "starting"]),
    services: z.array(serviceStatusSchema).length(3),
    version: z.string().min(1).max(64),
  })
  .strict();
export type StudioSnapshot = z.infer<typeof studioSnapshotSchema>;

export const workspaceSchema = z
  .object({
    id: workspaceIdSchema,
    name: z.string().min(1).max(LIMITS.labelCharacters),
    revision: z.string().min(1).max(128),
    rootEntryId: entryIdSchema,
  })
  .strict();
export type Workspace = z.infer<typeof workspaceSchema>;

export const fileEntrySchema = z
  .object({
    id: entryIdSchema,
    parentId: entryIdSchema.optional(),
    name: z.string().min(1).max(LIMITS.labelCharacters),
    kind: z.enum(["file", "directory"]),
    sizeBytes: z.number().int().min(0).max(Number.MAX_SAFE_INTEGER),
    language: z.string().min(1).max(64).optional(),
    revision: z.string().min(1).max(128).optional(),
    readOnly: z.boolean(),
  })
  .strict();
export type FileEntry = z.infer<typeof fileEntrySchema>;

export const filePageSchema = z
  .object({
    workspaceId: workspaceIdSchema,
    parentId: entryIdSchema.optional(),
    entries: z.array(fileEntrySchema).max(LIMITS.filePage),
    nextCursor: cursorSchema.optional(),
  })
  .strict();
export type FilePage = z.infer<typeof filePageSchema>;

export const fileDocumentSchema = z
  .object({
    workspaceId: workspaceIdSchema,
    entryId: entryIdSchema,
    name: z.string().min(1).max(LIMITS.labelCharacters),
    content: boundedUtf8(LIMITS.fileBytes),
    revision: z.string().min(1).max(128),
    language: z.string().min(1).max(64),
    readOnly: z.boolean(),
    binary: z.boolean(),
    truncated: z.boolean(),
  })
  .strict();
export type FileDocument = z.infer<typeof fileDocumentSchema>;

export const runStateSchema = z.enum([
  "queued",
  "running",
  "recovering",
  "succeeded",
  "failed",
  "cancelled",
]);

export const runSchema = z
  .object({
    id: runIdSchema,
    workspaceId: workspaceIdSchema,
    state: runStateSchema,
    lastSequence: sequenceSchema,
    updatedAt: utcTimestampSchema,
    detail: z.string().max(LIMITS.diagnosticCharacters).optional(),
  })
  .strict();
export type Run = z.infer<typeof runSchema>;

const runEventBodySchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("started"), label: z.string().max(256) }).strict(),
  z
    .object({
      kind: z.literal("stage"),
      stageId: opaqueId,
      label: z.string().max(256),
      state: z.string().min(1).max(64),
    })
    .strict(),
  z
    .object({
      kind: z.literal("output"),
      stream: z.enum(["stdout", "stderr", "system"]),
      data: boundedUtf8(LIMITS.terminalChunkBytes),
      truncated: z.boolean(),
    })
    .strict(),
  z
    .object({
      kind: z.literal("diagnostic"),
      code: z.string().min(1).max(96),
      message: z.string().max(LIMITS.diagnosticCharacters),
    })
    .strict(),
  z
    .object({
      kind: z.literal("finished"),
      state: runStateSchema,
      summary: z.string().max(LIMITS.diagnosticCharacters).optional(),
    })
    .strict(),
]);

export const runEventSchema = z
  .object({
    runId: runIdSchema,
    sequence: sequenceSchema,
    occurredAt: utcTimestampSchema,
    body: runEventBodySchema,
  })
  .strict();
export type RunEvent = z.infer<typeof runEventSchema>;

export const runStreamMessageSchema = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("events"),
      subscriptionId: subscriptionIdSchema,
      events: z.array(runEventSchema).min(1).max(LIMITS.runBatchEvents),
    })
    .strict(),
  z
    .object({
      kind: z.literal("resync-required"),
      subscriptionId: subscriptionIdSchema,
      lastSafeSequence: sequenceSchema,
      reason: z.enum(["gap", "backpressure", "retention"]),
    })
    .strict(),
  z
    .object({
      kind: z.literal("closed"),
      subscriptionId: subscriptionIdSchema,
      error: studioErrorSchema.optional(),
    })
    .strict(),
]);
export type RunStreamMessage = z.infer<typeof runStreamMessageSchema>;

export const scheduleSchema = z
  .object({
    id: scheduleIdSchema,
    taskId: opaqueId,
    label: z.string().min(1).max(LIMITS.labelCharacters),
    cron: z.string().min(1).max(128),
    timezone: z.string().min(1).max(128),
    state: z.enum(["disabled", "ready", "dispatching", "recovery-required"]),
    nextFireAt: utcTimestampSchema.optional(),
    lastRunId: runIdSchema.optional(),
  })
  .strict();
export type Schedule = z.infer<typeof scheduleSchema>;

export const schedulePageSchema = z
  .object({
    schedules: z.array(scheduleSchema).max(LIMITS.filePage),
    nextCursor: cursorSchema.optional(),
  })
  .strict();
export type SchedulePage = z.infer<typeof schedulePageSchema>;

export const recoverySchema = z
  .object({
    id: recoveryIdSchema,
    workspaceId: workspaceIdSchema,
    label: z.string().min(1).max(LIMITS.labelCharacters),
    state: z.enum(["available", "applied", "conflicted", "expired"]),
    createdAt: utcTimestampSchema,
    changedFiles: z.number().int().min(0).max(1_000_000),
    detail: z.string().max(LIMITS.diagnosticCharacters).optional(),
  })
  .strict();
export type Recovery = z.infer<typeof recoverySchema>;

export const recoveryPageSchema = z
  .object({
    records: z.array(recoverySchema).max(LIMITS.filePage),
    nextCursor: cursorSchema.optional(),
  })
  .strict();
export type RecoveryPage = z.infer<typeof recoveryPageSchema>;

export const terminalProfileSchema = z
  .object({
    id: z.enum(["powershell", "bash"]),
    label: z.string().min(1).max(64),
    available: z.boolean(),
  })
  .strict();
export type TerminalProfile = z.infer<typeof terminalProfileSchema>;

export const terminalSessionSchema = z
  .object({
    id: terminalIdSchema,
    profileId: terminalProfileSchema.shape.id,
  })
  .strict();
export type TerminalSession = z.infer<typeof terminalSessionSchema>;

export const terminalStreamMessageSchema = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("output"),
      terminalId: terminalIdSchema,
      sequence: sequenceSchema,
      data: boundedUtf8(LIMITS.terminalChunkBytes),
    })
    .strict(),
  z
    .object({
      kind: z.literal("exit"),
      terminalId: terminalIdSchema,
      exitCode: z.number().int().nullable(),
      reason: z.enum(["exited", "closed", "output-backpressure", "broker-stopped"]),
    })
    .strict(),
]);
export type TerminalStreamMessage = z.infer<typeof terminalStreamMessageSchema>;

export interface StreamHandle {
  readonly subscriptionId: SubscriptionId;
  unsubscribe(): Promise<void>;
}

export interface RunStreamHandle extends StreamHandle {
  ack(highestSequence: Sequence): Promise<void>;
}

export interface MotivoBridge {
  readonly surface: {
    current(): Promise<StudioSurface>;
    subscribe(listener: (surface: StudioSurface) => void): () => void;
  };
  readonly system: {
    snapshot(): Promise<StudioSnapshot>;
  };
  readonly workspaces: {
    open(): Promise<Workspace | null>;
  };
  readonly files: {
    listPage(input: unknown): Promise<FilePage>;
    read(input: unknown): Promise<FileDocument>;
    save(input: unknown): Promise<FileDocument>;
  };
  readonly runs: {
    start(input: unknown): Promise<Run>;
    get(input: unknown): Promise<Run>;
    cancel(input: unknown): Promise<Run>;
    subscribe(
      input: unknown,
      listener: (message: RunStreamMessage) => void,
    ): Promise<RunStreamHandle>;
  };
  readonly schedules: {
    listPage(input: unknown): Promise<SchedulePage>;
  };
  readonly recovery: {
    listPage(input: unknown): Promise<RecoveryPage>;
    apply(input: unknown): Promise<Recovery>;
  };
  readonly terminals: {
    profiles(): Promise<readonly TerminalProfile[]>;
    create(input: unknown): Promise<TerminalSession>;
    write(input: unknown): Promise<void>;
    resize(input: unknown): Promise<void>;
    ack(input: unknown): Promise<void>;
    close(input: unknown): Promise<void>;
    subscribe(
      input: unknown,
      listener: (message: TerminalStreamMessage) => void,
    ): Promise<StreamHandle>;
  };
}
