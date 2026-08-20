import { z } from "zod";
import { decimalStringSchema, LIMITS, utf8Bytes, workspaceHandleSchema } from "./contracts";

export const SESSION_LIMITS = {
  briefFindings: 12,
  findingSummaryCharacters: 400,
  findingDetailCharacters: 4_096,
  optionCount: 6,
  optionLabelCharacters: 160,
  coordinateCount: 12,
  coordinateCharacters: 64,
  promptCharacters: 1_024,
  consequenceCharacters: 1_024,
  noteBytes: 4_096,
  remainingAxes: 64,
  sessionIdCharacters: 128,
  answeredAxes: 256,
  sessionPage: 200,
} as const;

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

const optionIdSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/, {
    message:
      "Option ids must start with an alphanumeric character and contain only alphanumerics, '.', '_', or '-'.",
  });
const coordinateTextSchema = boundedText(SESSION_LIMITS.coordinateCharacters).pipe(
  z.string().min(1),
);

export const sessionIdSchema = z
  .string()
  .min(1)
  .max(SESSION_LIMITS.sessionIdCharacters)
  .regex(/^session-[A-Za-z0-9-]+$/);

export const axisIdSchema = z
  .string()
  .min(1)
  .max(128)
  .regex(/^[a-z][a-z0-9]*(\.[a-z0-9-]+)*$/);

export const reversibilitySchema = z.enum(["reversible", "costly", "irreversible"]);
export type Reversibility = z.infer<typeof reversibilitySchema>;

export const sessionFindingSchema = z
  .object({
    summary: boundedText(SESSION_LIMITS.findingSummaryCharacters).pipe(z.string().min(1)),
    detail: boundedText(SESSION_LIMITS.findingDetailCharacters).optional(),
    source: boundedText(SESSION_LIMITS.optionLabelCharacters).optional(),
  })
  .strict();
export type SessionFinding = z.infer<typeof sessionFindingSchema>;

export const sessionOptionSchema = z
  .object({
    id: optionIdSchema,
    label: boundedText(SESSION_LIMITS.optionLabelCharacters).pipe(z.string().min(1)),
    coordinates: z
      .record(coordinateTextSchema, coordinateTextSchema)
      .refine((value) => Object.keys(value).length <= SESSION_LIMITS.coordinateCount, {
        message: "Too many coordinates.",
      }),
    rationale: boundedText(SESSION_LIMITS.consequenceCharacters).optional(),
  })
  .strict();
export type SessionOption = z.infer<typeof sessionOptionSchema>;

export const sessionConsequenceSchema = z
  .object({
    option: optionIdSchema,
    effect: boundedText(SESSION_LIMITS.consequenceCharacters).pipe(z.string().min(1)),
    reversibility: reversibilitySchema,
  })
  .strict();
export type SessionConsequence = z.infer<typeof sessionConsequenceSchema>;

export const sessionQuestionSchema = z
  .object({
    axis: axisIdSchema,
    prompt: boundedText(SESSION_LIMITS.promptCharacters).pipe(z.string().min(1)),
    options: z.array(sessionOptionSchema).min(2).max(SESSION_LIMITS.optionCount),
    reversibility: reversibilitySchema,
    dependsOn: z.array(axisIdSchema).max(SESSION_LIMITS.remainingAxes),
  })
  .strict()
  .superRefine((value, context) => {
    addDuplicateIssues(
      value.options.map((option) => option.id),
      context,
      ["options"],
      "Option ids must be unique.",
    );
    addDuplicateIssues(value.dependsOn, context, ["dependsOn"], "Dependency axes must be unique.");
  });
export type SessionQuestion = z.infer<typeof sessionQuestionSchema>;

export const sessionBriefSchema = z
  .object({
    api: z.literal("agenstro.session/v1"),
    sessionId: sessionIdSchema,
    turn: decimalStringSchema,
    findings: z.array(sessionFindingSchema).max(SESSION_LIMITS.briefFindings),
    question: sessionQuestionSchema,
    stakes: z.array(sessionConsequenceSchema).max(SESSION_LIMITS.optionCount),
    defaultOption: optionIdSchema.optional(),
    remainingSurface: z.array(axisIdSchema).max(SESSION_LIMITS.remainingAxes),
    remainingFloor: z.array(axisIdSchema).max(SESSION_LIMITS.remainingAxes),
  })
  .strict()
  .superRefine((value, context) => {
    const options = new Set(value.question.options.map((option) => option.id));
    if (value.defaultOption !== undefined && !options.has(value.defaultOption)) {
      context.addIssue({
        code: "custom",
        path: ["defaultOption"],
        message: "The default must name an option in the pending question.",
      });
    }
    value.stakes.forEach((stake, index) => {
      if (!options.has(stake.option)) {
        context.addIssue({
          code: "custom",
          path: ["stakes", index, "option"],
          message: "A consequence must name an option in the pending question.",
        });
      }
    });
    addDuplicateIssues(
      value.remainingSurface,
      context,
      ["remainingSurface"],
      "Remaining surface axes must be unique.",
    );
    addDuplicateIssues(
      value.remainingFloor,
      context,
      ["remainingFloor"],
      "Remaining floor axes must be unique.",
    );
    const surface = new Set(value.remainingSurface);
    value.remainingFloor.forEach((axis, index) => {
      if (!surface.has(axis)) {
        context.addIssue({
          code: "custom",
          path: ["remainingFloor", index],
          message: "The required question floor must be contained in the question surface.",
        });
      }
    });
  });
export type SessionBrief = z.infer<typeof sessionBriefSchema>;

export const answeredAxisSchema = z
  .object({
    axis: axisIdSchema,
    option: optionIdSchema,
    label: boundedText(SESSION_LIMITS.optionLabelCharacters).pipe(z.string().min(1)),
    defaulted: z.boolean(),
    answeredAtUnixMs: decimalStringSchema,
  })
  .strict();
export type AnsweredAxis = z.infer<typeof answeredAxisSchema>;

export const sessionStateSchema = z.enum(["planning", "awaiting_answer", "delivered", "abandoned"]);
export type SessionState = z.infer<typeof sessionStateSchema>;

export const sessionViewSchema = z
  .object({
    api: z.literal("agenstro.session/v1"),
    sessionId: sessionIdSchema,
    label: boundedText(LIMITS.labelCharacters).pipe(z.string().min(1)),
    state: sessionStateSchema,
    turn: decimalStringSchema,
    pending: sessionBriefSchema.optional(),
    answered: z.array(answeredAxisSchema).max(SESSION_LIMITS.answeredAxes),
    startedUnixMs: decimalStringSchema,
    updatedUnixMs: decimalStringSchema,
  })
  .strict()
  .superRefine((value, context) => {
    if ((value.state === "awaiting_answer") !== (value.pending !== undefined)) {
      context.addIssue({
        code: "custom",
        path: ["pending"],
        message: "A pending brief is present exactly when the session awaits an answer.",
      });
    }
    if (value.pending && value.pending.sessionId !== value.sessionId) {
      context.addIssue({
        code: "custom",
        path: ["pending", "sessionId"],
        message: "The pending brief must belong to its containing session.",
      });
    }
    if (value.pending && value.pending.turn !== value.turn) {
      context.addIssue({
        code: "custom",
        path: ["pending", "turn"],
        message: "The pending brief turn must match its containing session.",
      });
    }
    const started = BigInt(value.startedUnixMs);
    const updated = BigInt(value.updatedUnixMs);
    if (updated < started) {
      context.addIssue({
        code: "custom",
        path: ["updatedUnixMs"],
        message: "The session update time must not precede its start time.",
      });
    }
    value.answered.forEach((answer, index) => {
      const answeredAt = BigInt(answer.answeredAtUnixMs);
      if (answeredAt < started || answeredAt > updated) {
        context.addIssue({
          code: "custom",
          path: ["answered", index, "answeredAtUnixMs"],
          message: "An answer time must fall between the session start and update times.",
        });
      }
    });
    addDuplicateIssues(
      value.answered.map((answer) => answer.axis),
      context,
      ["answered"],
      "Answered axes must be unique in the current right-biased projection.",
    );
  });
export type SessionView = z.infer<typeof sessionViewSchema>;

export const sessionAnswerInputSchema = z
  .object({
    workspaceHandle: workspaceHandleSchema,
    sessionId: sessionIdSchema,
    turn: decimalStringSchema,
    axis: axisIdSchema,
    option: optionIdSchema,
    note: boundedUtf8(SESSION_LIMITS.noteBytes)
      .refine((value) => !value.includes("\0"), { message: "Notes must not contain NUL." })
      .optional(),
  })
  .strict();
export type SessionAnswerInput = z.infer<typeof sessionAnswerInputSchema>;

export const sessionCurrentInputSchema = z
  .object({ workspaceHandle: workspaceHandleSchema, sessionId: sessionIdSchema })
  .strict();
export type SessionCurrentInput = z.infer<typeof sessionCurrentInputSchema>;

export const sessionListInputSchema = z
  .object({
    workspaceHandle: workspaceHandleSchema,
    limit: z.number().int().min(1).max(SESSION_LIMITS.sessionPage).optional(),
  })
  .strict();
export type SessionListInput = z.infer<typeof sessionListInputSchema>;

export const sessionListSchema = z
  .object({
    api: z.literal("agenstro.session/v1"),
    sessions: z.array(sessionViewSchema).max(SESSION_LIMITS.sessionPage),
  })
  .strict();
export type SessionList = z.infer<typeof sessionListSchema>;

export interface SessionBridge {
  list(input: SessionListInput): Promise<SessionList>;
  current(input: SessionCurrentInput): Promise<SessionView>;
  answer(input: SessionAnswerInput): Promise<SessionView>;
}

function addDuplicateIssues(
  values: readonly string[],
  context: z.core.$RefinementCtx<unknown>,
  path: PropertyKey[],
  message: string,
): void {
  const seen = new Set<string>();
  values.forEach((value, index) => {
    if (seen.has(value)) {
      context.addIssue({ code: "custom", path: [...path, index], message });
    }
    seen.add(value);
  });
}
