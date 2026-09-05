import { z } from "zod";

// These are Motivo's method records, not Clef types or a task-correctness contract.
const text = (limit: number) =>
  z
    .string()
    .max(limit)
    .refine((value) => !value.includes("\0"));
const nonempty = (limit: number) => text(limit).pipe(z.string().trim().min(1));
export const taskIdSchema = z.uuid();
export const taskActionSchema = z.enum(["investigate", "try", "integrate", "conclude"]);
export type TaskAction = z.infer<typeof taskActionSchema>;
export const taskStatusSchema = z.enum([
  "ready",
  "running",
  "paused",
  "needs_input",
  "completed",
  "failed",
  "outcome_unknown",
]);

export const taskFindingSchema = z
  .object({
    statement: nonempty(4000),
    source: text(2000).optional(),
  })
  .strict();
export const taskCheckSchema = z
  .object({
    name: nonempty(256),
    result: z.enum(["passed", "failed", "unknown"]),
    detail: text(4000),
    source: text(2000).optional(),
  })
  .strict();

export const taskReportSchema = z
  .object({
    action: taskActionSchema,
    focus: nonempty(4000),
    summary: nonempty(8000),
    findings: z.array(taskFindingSchema).max(30),
    unknowns: z.array(nonempty(2000)).max(20),
    decision: nonempty(4000),
    artifacts: z.array(nonempty(2000)).max(30),
    checks: z.array(taskCheckSchema).max(30),
    next: text(4000),
    status: z.enum(["continue", "needs_input", "completed"]),
    question: nonempty(4000).optional(),
    // Optional questions, not a mandatory plan or permission to modify concurrently.
    investigations: z.array(nonempty(2000)).max(3).optional(),
  })
  .strict()
  .superRefine((report, context) => {
    if (report.status === "needs_input" && !report.question) {
      context.addIssue({
        code: "custom",
        message: "A question is required when user input is needed.",
      });
    }
    if (
      report.investigations?.length &&
      (report.action !== "investigate" || report.status !== "continue")
    ) {
      context.addIssue({
        code: "custom",
        message: "Investigation branches require a continuing investigation.",
      });
    }
  });
export type TaskReport = z.infer<typeof taskReportSchema>;

export const taskRoundSchema = z
  .object({
    id: z.uuid(),
    role: z.enum(["lead", "investigator"]),
    focus: nonempty(4000),
    startedAt: z.string().datetime(),
    finishedAt: z.string().datetime().optional(),
    elapsedMs: z.number().int().nonnegative().optional(),
    runId: text(128).optional(),
    outcome: z.enum(["running", "succeeded", "failed", "outcome_unknown"]),
    report: taskReportSchema.optional(),
    error: text(4000).optional(),
    // Failed report decoding retains only a bounded diagnostic excerpt.
    rawOutput: z.string().max(8000).optional(),
    rawOutputTruncated: z.boolean().optional(),
  })
  .strict();
export type TaskRound = z.infer<typeof taskRoundSchema>;

export const taskDocumentSchema = z
  .object({
    api: z.literal("motivo.task/v1"),
    id: taskIdSchema,
    goal: nonempty(16000),
    constraints: text(8000),
    provider: z
      .string()
      .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/)
      .max(128),
    status: taskStatusSchema,
    createdAt: z.string().datetime(),
    updatedAt: z.string().datetime(),
    revision: z.number().int().nonnegative(),
    calls: z.number().int().nonnegative(),
    pauseRequested: z.boolean(),
    notes: z.array(z.object({ text: nonempty(8000), at: z.string().datetime() }).strict()).max(100),
    rounds: z.array(taskRoundSchema).max(200),
    message: text(4000).optional(),
  })
  .strict();
export type TaskDocument = z.infer<typeof taskDocumentSchema>;

export const taskSummarySchema = taskDocumentSchema.pick({
  id: true,
  goal: true,
  provider: true,
  status: true,
  updatedAt: true,
  calls: true,
});
export type TaskSummary = z.infer<typeof taskSummarySchema>;
export const taskListSchema = z.array(taskSummarySchema).max(50);
export const taskWorkspaceInputSchema = z.object({ workspaceHandle: z.uuid() }).strict();
export const taskCurrentInputSchema = taskWorkspaceInputSchema
  .extend({ taskId: taskIdSchema })
  .strict();
export const taskCreateInputSchema = taskWorkspaceInputSchema
  .extend({
    goal: nonempty(16000),
    constraints: text(8000).default(""),
    provider: z
      .string()
      .regex(/^[A-Za-z0-9][A-Za-z0-9._-]*$/)
      .max(128),
  })
  .strict();
export const taskContinueInputSchema = taskCurrentInputSchema
  .extend({
    // Budget counts all provider calls, including parallel investigations.
    maxCalls: z.number().int().min(1).max(20).default(4),
    note: nonempty(8000).optional(),
  })
  .strict();
export type TaskCreateInput = z.infer<typeof taskCreateInputSchema>;
export type TaskContinueInput = z.infer<typeof taskContinueInputSchema>;
export type TaskCurrentInput = z.infer<typeof taskCurrentInputSchema>;

export interface TaskBridge {
  list(input: { readonly workspaceHandle: string }): Promise<TaskSummary[]>;
  current(input: TaskCurrentInput): Promise<TaskDocument>;
  create(input: TaskCreateInput): Promise<TaskDocument>;
  continue(input: TaskContinueInput): Promise<TaskDocument>;
  pause(input: TaskCurrentInput): Promise<TaskDocument>;
}
