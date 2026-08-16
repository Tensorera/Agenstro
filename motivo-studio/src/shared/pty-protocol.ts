import { z } from "zod";
import {
  LIMITS,
  sequenceSchema,
  terminalIdSchema,
  terminalProfileSchema,
  utf8Bytes,
} from "./contracts";

const commandIdSchema = z.uuid();
const nativePathSchema = z
  .string()
  .min(1)
  .max(32_767)
  .refine((value) => !value.includes("\0"));

export const ptyCommandSchema = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("create"),
      commandId: commandIdSchema,
      terminalId: terminalIdSchema,
      profileId: terminalProfileSchema.shape.id,
      cwd: nativePathSchema,
      cols: z.number().int().min(20).max(500),
      rows: z.number().int().min(5).max(200),
    })
    .strict(),
  z
    .object({
      kind: z.literal("write"),
      terminalId: terminalIdSchema,
      data: z.string().refine((value) => utf8Bytes(value) <= LIMITS.terminalInputBytes),
    })
    .strict(),
  z
    .object({
      kind: z.literal("resize"),
      terminalId: terminalIdSchema,
      cols: z.number().int().min(20).max(500),
      rows: z.number().int().min(5).max(200),
    })
    .strict(),
  z
    .object({ kind: z.literal("ack"), terminalId: terminalIdSchema, sequence: sequenceSchema })
    .strict(),
  z.object({ kind: z.literal("close"), terminalId: terminalIdSchema }).strict(),
  z.object({ kind: z.literal("shutdown") }).strict(),
]);
export type PtyCommand = z.infer<typeof ptyCommandSchema>;

export const ptyEventSchema = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("created"),
      commandId: commandIdSchema,
      terminalId: terminalIdSchema,
    })
    .strict(),
  z
    .object({
      kind: z.literal("output"),
      terminalId: terminalIdSchema,
      sequence: sequenceSchema,
      data: z.string().refine((value) => utf8Bytes(value) <= LIMITS.terminalChunkBytes),
      bytes: z.number().int().min(0).max(LIMITS.terminalChunkBytes),
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
  z
    .object({
      kind: z.literal("error"),
      commandId: commandIdSchema.optional(),
      terminalId: terminalIdSchema.optional(),
      code: z.string().min(1).max(96),
      message: z.string().min(1).max(1_024),
    })
    .strict(),
]);
export type PtyEvent = z.infer<typeof ptyEventSchema>;

export const ptyHostEventSchema = z.union([
  ptyEventSchema,
  z.object({ kind: z.literal("stopped") }).strict(),
]);
export type PtyHostEvent = z.infer<typeof ptyHostEventSchema>;
