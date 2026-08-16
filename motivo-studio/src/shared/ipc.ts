import { z } from "zod";
import {
  cursorSchema,
  entryIdSchema,
  LIMITS,
  pageSizeSchema,
  recoveryIdSchema,
  requestIdSchema,
  runIdSchema,
  sequenceSchema,
  subscriptionIdSchema,
  terminalIdSchema,
  terminalProfileSchema,
  utf8Bytes,
  workspaceIdSchema,
} from "./contracts";

export const IPC = {
  surfaceCurrent: "motivo:surface:current",
  surfaceChanged: "motivo:surface:changed",
  systemSnapshot: "motivo:system:snapshot",
  workspaceOpen: "motivo:workspace:open",
  filesListPage: "motivo:files:list-page",
  filesRead: "motivo:files:read",
  filesSave: "motivo:files:save",
  runsStart: "motivo:runs:start",
  runsGet: "motivo:runs:get",
  runsCancel: "motivo:runs:cancel",
  runsSubscribe: "motivo:runs:subscribe",
  runsUnsubscribe: "motivo:runs:unsubscribe",
  runsAck: "motivo:runs:ack",
  runsEvent: "motivo:runs:event",
  schedulesListPage: "motivo:schedules:list-page",
  recoveryListPage: "motivo:recovery:list-page",
  recoveryApply: "motivo:recovery:apply",
  terminalsProfiles: "motivo:terminals:profiles",
  terminalsCreate: "motivo:terminals:create",
  terminalsWrite: "motivo:terminals:write",
  terminalsResize: "motivo:terminals:resize",
  terminalsAck: "motivo:terminals:ack",
  terminalsClose: "motivo:terminals:close",
  terminalsSubscribe: "motivo:terminals:subscribe",
  terminalsUnsubscribe: "motivo:terminals:unsubscribe",
  terminalsEvent: "motivo:terminals:event",
} as const;

export const emptyInputSchema = z.object({}).strict();
export const pageInputSchema = z
  .object({ pageSize: pageSizeSchema, cursor: cursorSchema.optional() })
  .strict();
export const filePageInputSchema = pageInputSchema
  .extend({ workspaceId: workspaceIdSchema, parentId: entryIdSchema.optional() })
  .strict();
export const fileReadInputSchema = z
  .object({ workspaceId: workspaceIdSchema, entryId: entryIdSchema })
  .strict();
export const fileSaveInputSchema = z
  .object({
    requestId: requestIdSchema,
    workspaceId: workspaceIdSchema,
    entryId: entryIdSchema,
    expectedRevision: z.string().min(1).max(128),
    content: z.string().refine((value) => utf8Bytes(value) <= LIMITS.fileBytes),
  })
  .strict();
export const runStartInputSchema = z
  .object({
    requestId: requestIdSchema,
    workspaceId: workspaceIdSchema,
    entryId: entryIdSchema,
  })
  .strict();
export const runGetInputSchema = z.object({ runId: runIdSchema }).strict();
export const runCancelInputSchema = z
  .object({ requestId: requestIdSchema, runId: runIdSchema })
  .strict();
export const runSubscribeInputSchema = z
  .object({
    subscriptionId: subscriptionIdSchema,
    runId: runIdSchema,
    afterSequence: sequenceSchema,
  })
  .strict();
export const runSubscribeRequestSchema = runSubscribeInputSchema
  .omit({ subscriptionId: true })
  .strict();
export const streamUnsubscribeInputSchema = z
  .object({ subscriptionId: subscriptionIdSchema })
  .strict();
export const runAckInputSchema = z
  .object({ subscriptionId: subscriptionIdSchema, highestSequence: sequenceSchema })
  .strict();
export const recoveryPageInputSchema = pageInputSchema
  .extend({ workspaceId: workspaceIdSchema })
  .strict();
export const recoveryApplyInputSchema = z
  .object({
    requestId: requestIdSchema,
    workspaceId: workspaceIdSchema,
    recoveryId: recoveryIdSchema,
  })
  .strict();
export const terminalCreateInputSchema = z
  .object({
    workspaceId: workspaceIdSchema,
    profileId: terminalProfileSchema.shape.id,
    cols: z.number().int().min(20).max(500),
    rows: z.number().int().min(5).max(200),
  })
  .strict();
export const terminalWriteInputSchema = z
  .object({
    terminalId: terminalIdSchema,
    data: z.string().refine((value) => utf8Bytes(value) <= LIMITS.terminalInputBytes),
  })
  .strict();
export const terminalResizeInputSchema = z
  .object({
    terminalId: terminalIdSchema,
    cols: z.number().int().min(20).max(500),
    rows: z.number().int().min(5).max(200),
  })
  .strict();
export const terminalAckInputSchema = z
  .object({ terminalId: terminalIdSchema, highestSequence: sequenceSchema })
  .strict();
export const terminalCloseInputSchema = z.object({ terminalId: terminalIdSchema }).strict();
export const terminalSubscribeInputSchema = z
  .object({ subscriptionId: subscriptionIdSchema, terminalId: terminalIdSchema })
  .strict();
export const terminalSubscribeRequestSchema = terminalSubscribeInputSchema
  .omit({ subscriptionId: true })
  .strict();
