import { z } from "zod";
import type { SessionBridge } from "./session-contracts";

export const LIMITS = {
  actionOutputBytes: 16_384,
  diagnosticCharacters: 4_096,
  eventPage: 250,
  generationGoalBytes: 32_768,
  labelCharacters: 256,
  pluginSelection: 100,
  runIdCharacters: 128,
} as const;

const encoder = new TextEncoder();

export function utf8Bytes(value: string): number {
  return encoder.encode(value).byteLength;
}

function boundedText(characters: number) {
  return z.string().refine((value) => [...value].length <= characters, {
    message: `Value exceeds the ${String(characters)} character limit.`,
  });
}

function boundedUtf8(bytes: number) {
  return z.string().refine((value) => utf8Bytes(value) <= bytes, {
    message: `Value exceeds the ${String(bytes)} byte limit.`,
  });
}

export const decimalStringSchema = z
  .string()
  .regex(/^(0|[1-9][0-9]*)$/)
  .refine(
    (value) => {
      try {
        return BigInt(value) <= 18_446_744_073_709_551_615n;
      } catch {
        return false;
      }
    },
    { message: "Value exceeds the unsigned 64-bit range." },
  );
export const actionIdSchema = z.uuid();
export const workspaceHandleSchema = z.uuid();
export const runIdSchema = z
  .string()
  .min(1)
  .max(LIMITS.runIdCharacters)
  .regex(/^run-[A-Za-z0-9-]+$/);
export const registryNameSchema = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/);

export const studioErrorSchema = z
  .object({
    code: z
      .string()
      .min(1)
      .max(96)
      .regex(/^[a-z][a-z0-9_]*$/),
    category: z.enum(["validation", "process", "workspace", "busy", "cancelled", "internal"]),
    retryable: z.boolean(),
    message: boundedText(LIMITS.diagnosticCharacters),
  })
  .strict();
export type StudioError = z.infer<typeof studioErrorSchema>;

export type IpcResult<T> =
  { readonly ok: true; readonly data: T } | { readonly ok: false; readonly error: StudioError };

export const ipcEnvelopeSchema = z.discriminatedUnion("ok", [
  z.object({ ok: z.literal(true), data: z.unknown() }).strict(),
  z.object({ ok: z.literal(false), error: studioErrorSchema }).strict(),
]);

export const studioCheckSchema = z
  .object({
    name: boundedText(128),
    ok: z.boolean(),
    detail: boundedText(LIMITS.diagnosticCharacters),
  })
  .strict();

export const studioScriptSchema = z
  .object({
    relativePath: z.string().min(1).max(1_024),
    order: z.number().int().min(0).max(999).optional(),
    runnable: z.boolean(),
  })
  .strict();
export type StudioScript = z.infer<typeof studioScriptSchema>;

export const pluginNamespaceSchema = z.enum(["provider", "effect", "plugin"]);
export type PluginNamespace = z.infer<typeof pluginNamespaceSchema>;

export const studioPluginSchema = z
  .object({
    name: registryNameSchema,
    namespace: pluginNamespaceSchema,
    available: z.boolean(),
    default: z.boolean(),
    model: boundedText(LIMITS.labelCharacters).optional(),
    effort: boundedText(LIMITS.labelCharacters).optional(),
    observesInvocations: z.boolean(),
  })
  .strict();
export type StudioPlugin = z.infer<typeof studioPluginSchema>;

export const studioOutcomeSchema = z
  .object({
    kind: boundedText(64),
    exitCode: z.number().int().optional(),
    error: boundedText(LIMITS.diagnosticCharacters).optional(),
    elapsedMs: decimalStringSchema,
    stderrTruncated: z.boolean(),
  })
  .strict();
export type StudioOutcome = z.infer<typeof studioOutcomeSchema>;

export const studioIntegritySchema = z.enum(["ok", "partial", "corrupt"]);

export const presentationCategorySchema = z.enum(["state", "info", "warning", "error"]);
export type PresentationCategory = z.infer<typeof presentationCategorySchema>;

export const studioPresentationSchema = z
  .object({
    category: presentationCategorySchema,
    message: boundedText(LIMITS.diagnosticCharacters).pipe(z.string().min(1)),
  })
  .strict();
export type StudioPresentation = z.infer<typeof studioPresentationSchema>;

export const studioRunSchema = z
  .object({
    runId: runIdSchema,
    state: boundedText(64),
    integrity: studioIntegritySchema,
    startedUnixMs: decimalStringSchema,
    finishedUnixMs: decimalStringSchema.optional(),
    eventsRecorded: decimalStringSchema,
    label: boundedText(LIMITS.labelCharacters),
    namespace: boundedText(32).optional(),
    subject: boundedText(128).optional(),
    method: boundedText(128).optional(),
    outcome: studioOutcomeSchema.optional(),
  })
  .strict();
export type StudioRun = z.infer<typeof studioRunSchema>;

export const studioSnapshotSchema = z
  .object({
    api: z.literal("agenstro.studio/v1"),
    generatedAtUnixMs: decimalStringSchema,
    workspace: z
      .object({ name: boundedText(LIMITS.labelCharacters).pipe(z.string().min(1)) })
      .strict(),
    health: z.object({ ok: z.boolean(), checks: z.array(studioCheckSchema).max(256) }).strict(),
    scripts: z.array(studioScriptSchema).max(10_000),
    registries: z
      .object({
        defaultProvider: registryNameSchema,
        providers: z.array(studioPluginSchema).max(1_000),
        effects: z.array(studioPluginSchema).max(1_000),
        plugins: z.array(studioPluginSchema).max(1_000),
      })
      .strict(),
    runs: z.array(studioRunSchema).max(200),
  })
  .strict();
export type StudioSnapshot = z.infer<typeof studioSnapshotSchema>;

export const studioViewSchema = z
  .object({
    handle: workspaceHandleSchema,
    snapshot: studioSnapshotSchema,
  })
  .strict();
export type StudioView = z.infer<typeof studioViewSchema>;

export const studioEventSchema = z
  .object({
    seq: decimalStringSchema,
    atUnixMs: decimalStringSchema,
    kind: boundedText(128),
    data: z.unknown(),
    presentation: studioPresentationSchema.optional(),
  })
  .strict();
export type StudioEvent = z.infer<typeof studioEventSchema>;

export const studioSummarySchema = z
  .object({
    startedUnixMs: decimalStringSchema,
    finishedUnixMs: decimalStringSchema,
    eventsRecorded: decimalStringSchema,
    outcome: studioOutcomeSchema,
  })
  .strict();

export const studioEventPageSchema = z
  .object({
    api: z.literal("agenstro.studio/v1"),
    run: studioRunSchema,
    events: z.array(studioEventSchema).max(1_000),
    nextAfter: decimalStringSchema,
    complete: z.boolean(),
    integrity: studioIntegritySchema,
    summary: studioSummarySchema.optional(),
  })
  .strict();
export type StudioEventPage = z.infer<typeof studioEventPageSchema>;

export const actionKindSchema = z.enum(["generate", "check", "run", "smoke"]);
export type ActionKind = z.infer<typeof actionKindSchema>;

export const actionRequestSchema = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("generate"),
      goal: boundedUtf8(LIMITS.generationGoalBytes).pipe(z.string().min(1)),
      provider: registryNameSchema.optional(),
    })
    .strict(),
  z.object({ kind: z.literal("check") }).strict(),
  z.object({ kind: z.literal("run") }).strict(),
  z
    .object({
      kind: z.literal("smoke"),
      targets: z
        .array(z.object({ namespace: pluginNamespaceSchema, name: registryNameSchema }).strict())
        .min(1)
        .max(LIMITS.pluginSelection),
      live: z.boolean(),
    })
    .strict(),
]);
export type ActionRequest = z.infer<typeof actionRequestSchema>;

export const actionStateSchema = z
  .object({
    actionId: actionIdSchema,
    kind: actionKindSchema,
    startedAtUnixMs: decimalStringSchema,
  })
  .strict();
export type ActionState = z.infer<typeof actionStateSchema>;

export const studioActionEventSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("started"),
      actionId: actionIdSchema,
      kind: actionKindSchema,
      startedAtUnixMs: decimalStringSchema,
    })
    .strict(),
  z
    .object({
      type: z.literal("output"),
      actionId: actionIdSchema,
      sequence: decimalStringSchema,
      stream: z.enum(["stdout", "stderr"]),
      text: boundedUtf8(LIMITS.actionOutputBytes),
      presentation: studioPresentationSchema.optional(),
    })
    .strict(),
  z
    .object({
      type: z.literal("finished"),
      actionId: actionIdSchema,
      sequence: decimalStringSchema,
      status: z.enum(["succeeded", "failed", "cancelled"]),
      exitCode: z.number().int().nullable(),
      finishedAtUnixMs: decimalStringSchema,
      message: boundedText(LIMITS.diagnosticCharacters).optional(),
    })
    .strict(),
]);
export type StudioActionEvent = z.infer<typeof studioActionEventSchema>;

export interface MotivoBridge {
  readonly studio: {
    current(): Promise<StudioView | null>;
    openInitialized(): Promise<StudioView | null>;
    initialize(): Promise<StudioView | null>;
    refresh(): Promise<StudioView>;
  };
  readonly actions: {
    start(input: ActionRequest): Promise<ActionState>;
    cancel(input: { readonly actionId: string }): Promise<void>;
    subscribe(listener: (event: StudioActionEvent) => void): () => void;
  };
  readonly runs: {
    events(input: {
      readonly runId: string;
      readonly after: string;
      readonly limit?: number;
    }): Promise<StudioEventPage>;
  };
  readonly sessions: SessionBridge;
}
