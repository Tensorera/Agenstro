import { describe, expect, it } from "vitest";
import {
  actionRequestSchema,
  decimalStringSchema,
  LIMITS,
  studioActionEventSchema,
  studioEventSchema,
  studioSnapshotSchema,
} from "../../src/shared/contracts";
import { runEventsInputSchema } from "../../src/shared/ipc";
import {
  taskContinueInputSchema,
  taskCreateInputSchema,
  taskReportSchema,
} from "../../src/shared/task-contracts";
import {
  SESSION_LIMITS,
  sessionAnswerInputSchema,
  sessionViewSchema,
} from "../../src/shared/session-contracts";

const workspaceHandle = "aa665bbe-ece0-40e6-8235-2278635aee84";

describe("bounded Motivo IPC contracts", () => {
  it("requires an explicit nonempty script selection for check and run", () => {
    for (const kind of ["check", "run"]) {
      expect(() => actionRequestSchema.parse({ kind })).toThrow();
      expect(() => actionRequestSchema.parse({ kind, scripts: [] })).toThrow();
      expect(
        actionRequestSchema.parse({ kind, scripts: [".tactus/scripts/010_main.hs"] }),
      ).toMatchObject({ kind });
    }
  });

  it("bounds task budgets and rejects extra workspace authority", () => {
    const input = { workspaceHandle, taskId: "bb665bbe-ece0-40e6-8235-2278635aee84" };
    expect(taskContinueInputSchema.parse(input).maxCalls).toBe(4);
    for (const maxCalls of [0, 21, 1.5]) {
      expect(() => taskContinueInputSchema.parse({ ...input, maxCalls })).toThrow();
    }
    expect(() => taskContinueInputSchema.parse({ ...input, root: "/other" })).toThrow();
    expect(() =>
      taskCreateInputSchema.parse({ workspaceHandle, goal: " ", provider: "codex" }),
    ).toThrow();
    expect(() =>
      taskCreateInputSchema.parse({ workspaceHandle, goal: "fix it", provider: "../codex" }),
    ).toThrow();
    expect(() => taskReportSchema.parse({ status: "completed", summary: "done" })).toThrow();
  });

  it("keeps JavaScript-unsafe counters as canonical decimal text", () => {
    expect(decimalStringSchema.parse("18446744073709551615")).toBe("18446744073709551615");
    for (const invalid of ["", "01", "-1", "1.0", " 1", "18446744073709551616"]) {
      expect(() => decimalStringSchema.parse(invalid)).toThrow();
    }
  });

  it("rejects unknown action authority and oversized generation goals", () => {
    expect(() => actionRequestSchema.parse({ kind: "run", root: "C:\\secrets" })).toThrow();
    expect(() =>
      actionRequestSchema.parse({
        kind: "generate",
        goal: "界".repeat(Math.floor(LIMITS.generationGoalBytes / 3) + 1),
      }),
    ).toThrow();
    expect(() =>
      actionRequestSchema.parse({
        kind: "smoke",
        targets: [{ namespace: "provider", name: "codex" }],
        live: false,
      }),
    ).not.toThrow();
  });

  it("accepts only bounded opaque event-page requests", () => {
    expect(() =>
      runEventsInputSchema.parse({ runId: "../events.jsonl", after: "0", limit: 10 }),
    ).toThrow();
    expect(() =>
      runEventsInputSchema.parse({
        runId: "run-123-42-0",
        after: "0",
        limit: LIMITS.eventPage + 1,
      }),
    ).toThrow();
  });

  it("does not admit an absolute workspace path into the Studio snapshot", () => {
    const valid = snapshot();
    expect(studioSnapshotSchema.parse(valid).workspace.name).toBe("sample");
    expect(() =>
      studioSnapshotSchema.parse({
        ...valid,
        workspace: { name: "sample", root: "D:\\private\\sample" },
      }),
    ).toThrow();
  });

  it("accepts only canonical presentation categories and keeps them optional", () => {
    const legacy = {
      seq: "1",
      atUnixMs: "1770000000000",
      kind: "legacy.event",
      data: { raw: true },
    };
    expect(studioEventSchema.parse(legacy).presentation).toBeUndefined();
    expect(
      studioEventSchema.parse({
        ...legacy,
        presentation: { category: "warning", message: "Evidence is incomplete." },
      }).presentation,
    ).toEqual({ category: "warning", message: "Evidence is incomplete." });
    expect(() =>
      studioEventSchema.parse({
        ...legacy,
        presentation: { category: "debug", message: "Not public." },
      }),
    ).toThrow();

    expect(() =>
      studioActionEventSchema.parse({
        type: "output",
        actionId: "3ce53087-2218-42fd-bdda-afc4097020ae",
        sequence: "1",
        stream: "stderr",
        text: "normal progress\n",
        presentation: { category: "error", message: "Only explicit tags project." },
      }),
    ).not.toThrow();
  });

  it("keeps session projections strict and enforces the pending-turn invariant", () => {
    const valid = sessionView();
    expect(sessionViewSchema.parse(valid).pending?.question.axis).toBe("desk.frame");
    expect(() => sessionViewSchema.parse({ ...valid, workspaceRoot: "D:\\private" })).toThrow();
    expect(() =>
      sessionViewSchema.parse({
        ...valid,
        pending: { ...(valid.pending as object), futureField: true },
      }),
    ).toThrow();
    expect(() => sessionViewSchema.parse({ ...valid, state: "planning" })).toThrow();
    expect(() =>
      sessionViewSchema.parse({
        ...valid,
        pending: { ...(valid.pending as object), turn: "4" },
      }),
    ).toThrow();
    expect(() => sessionViewSchema.parse({ ...valid, label: "" })).toThrow();
    expect(() =>
      sessionViewSchema.parse({ ...valid, startedUnixMs: "3", updatedUnixMs: "2" }),
    ).toThrow();
    expect(() =>
      sessionViewSchema.parse({
        ...valid,
        answered: [
          {
            axis: "desk.budget",
            option: "mid",
            label: "Mid-range",
            defaulted: false,
            answeredAtUnixMs: "3",
          },
        ],
      }),
    ).toThrow();
  });

  it("validates session defaults, roadmap bounds, and bounded inbound notes", () => {
    const valid = sessionView();
    expect(() =>
      sessionViewSchema.parse({
        ...valid,
        pending: { ...(valid.pending as object), defaultOption: "not-an-option" },
      }),
    ).toThrow();
    expect(() =>
      sessionViewSchema.parse({
        ...valid,
        pending: {
          ...(valid.pending as object),
          remainingFloor: ["desk.unreachable"],
        },
      }),
    ).toThrow();
    expect(() =>
      sessionAnswerInputSchema.parse({
        workspaceHandle,
        sessionId: "session-desk-1",
        turn: "3",
        axis: "desk.frame",
        option: "fixed",
        note: "界".repeat(Math.floor(SESSION_LIMITS.noteBytes / 3) + 1),
      }),
    ).toThrow();
    expect(() =>
      sessionAnswerInputSchema.parse({
        workspaceHandle,
        sessionId: "session-desk-1",
        turn: "3",
        axis: "desk.frame",
        option: "fixed",
        root: "D:\\private",
      }),
    ).toThrow();
    for (const option of ["-fixed", "../fixed", "fixed/child", "fixed\0child"]) {
      expect(() =>
        sessionAnswerInputSchema.parse({
          workspaceHandle,
          sessionId: "session-desk-1",
          turn: "3",
          axis: "desk.frame",
          option,
        }),
      ).toThrow();
    }
    expect(() =>
      sessionAnswerInputSchema.parse({
        workspaceHandle,
        sessionId: "session-desk-1",
        turn: "3",
        axis: "desk.frame",
        option: "fixed",
        note: "before\0after",
      }),
    ).toThrow();
  });
});

function snapshot(): Record<string, unknown> {
  return {
    api: "agenstro.studio/v1",
    generatedAtUnixMs: "1770000000000",
    workspace: { name: "sample" },
    health: { ok: true, checks: [] },
    scripts: [],
    registries: { defaultProvider: "codex", providers: [], effects: [], plugins: [] },
    runs: [],
  };
}

function sessionView(): Record<string, unknown> & { pending: Record<string, unknown> } {
  return {
    api: "agenstro.session/v1",
    sessionId: "session-desk-1",
    label: "Desk build",
    state: "awaiting_answer",
    turn: "3",
    pending: {
      api: "agenstro.session/v1",
      sessionId: "session-desk-1",
      turn: "3",
      findings: [{ summary: "A grounded finding.", source: "fixture corpus" }],
      question: {
        axis: "desk.frame",
        prompt: "Choose a frame.",
        options: [
          { id: "fixed", label: "Fixed", coordinates: { height: "fixed" } },
          { id: "moving", label: "Moving", coordinates: { height: "adjustable" } },
        ],
        reversibility: "irreversible",
        dependsOn: [],
      },
      stakes: [
        {
          option: "fixed",
          effect: "Commits the height.",
          reversibility: "irreversible",
        },
      ],
      remainingSurface: ["desk.frame", "desk.finish"],
      remainingFloor: ["desk.frame"],
    },
    answered: [],
    startedUnixMs: "1",
    updatedUnixMs: "2",
  };
}
