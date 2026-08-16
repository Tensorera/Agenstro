import { z } from "zod";
import {
  actionIdSchema,
  actionRequestSchema,
  decimalStringSchema,
  LIMITS,
  runIdSchema,
} from "./contracts";

export const IPC = {
  studioCurrent: "motivo:studio:current",
  studioOpenInitialized: "motivo:studio:open-initialized",
  studioInitialize: "motivo:studio:initialize",
  studioRefresh: "motivo:studio:refresh",
  actionStart: "motivo:action:start",
  actionCancel: "motivo:action:cancel",
  actionEvent: "motivo:action:event",
  runEvents: "motivo:run:events",
} as const;

export const emptyInputSchema = z.object({}).strict();
export const actionStartInputSchema = actionRequestSchema;
export const actionCancelInputSchema = z.object({ actionId: actionIdSchema }).strict();
export const runEventsInputSchema = z
  .object({
    runId: runIdSchema,
    after: decimalStringSchema,
    limit: z.number().int().min(1).max(LIMITS.eventPage).optional(),
  })
  .strict();
