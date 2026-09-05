import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_METHOD,
  loadMethod,
  parseTaskReport,
  taskPrompt,
} from "../../src/main/tasks/method";
import { TaskService } from "../../src/main/tasks/service";
import { TaskStore } from "../../src/main/tasks/store";
import {
  ProviderCallError,
  type ProviderInvocation,
  type ProviderResult,
} from "../../src/main/tasks/transport";
import {
  taskContinueInputSchema,
  type TaskDocument,
  type TaskReport,
  type TaskRound,
} from "../../src/shared/task-contracts";

const roots: string[] = [];
const services: TaskService[] = [];
const workspaceHandle = randomUUID();

afterEach(async () => {
  for (const service of services.splice(0)) {
    service.dispose();
    await service.waitForIdle();
  }
  vi.restoreAllMocks();
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

async function setup(invoke: (input: ProviderInvocation) => Promise<ProviderResult>) {
  const root = await mkdtemp(join(tmpdir(), "motivo-task-service-"));
  roots.push(root);
  const service = new TaskService({ invoke });
  services.push(service);
  const task = await service.create(root, {
    workspaceHandle,
    goal: "Fix the parser",
    constraints: "Keep the public API",
    provider: "fixture",
  });
  return { root, service, task };
}

function report(overrides: Partial<TaskReport> = {}): TaskReport {
  return {
    action: "integrate",
    focus: "Parser behavior",
    summary: "Updated the parser",
    findings: [],
    unknowns: [],
    decision: "Use the existing interface",
    artifacts: [],
    checks: [],
    next: "",
    status: "completed",
    ...overrides,
  };
}

function returned(value: TaskReport): ProviderResult {
  return { text: JSON.stringify(value) };
}

function proceed(taskId: string, note?: string, maxCalls = 4) {
  return taskContinueInputSchema.parse({
    workspaceHandle,
    taskId,
    maxCalls,
    ...(note ? { note } : {}),
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function savedRound(overrides: Partial<TaskRound> = {}): TaskRound {
  return {
    id: randomUUID(),
    role: "lead",
    focus: "Parser behavior",
    startedAt: new Date().toISOString(),
    outcome: "succeeded",
    report: report(),
    ...overrides,
  };
}

function context(prompt: string): {
  recentHistory: { report?: TaskReport }[];
  handoff?: { next: string; decision: string; question?: string };
  userNotes: { text: string }[];
  omittedEarlierNotes: number;
  omittedEarlierRounds: number;
  historySource: string;
} {
  const json = prompt.split("Task data:\n\n")[1]?.split("\n\n")[0];
  if (!json) throw new Error("Missing task context.");
  return JSON.parse(json) as ReturnType<typeof context>;
}

describe("task execution method", () => {
  it("persists creation without invoking a provider and restores task summaries", async () => {
    const invoke = vi.fn(async () => returned(report()));
    const { root, service, task } = await setup(invoke);
    expect(task.status).toBe("ready");
    expect(task.calls).toBe(0);
    expect(invoke).not.toHaveBeenCalled();
    const listing = vi.spyOn(TaskStore.prototype, "list");
    expect(await service.list(root)).toEqual([
      expect.objectContaining({ id: task.id, goal: task.goal, calls: 0, status: "ready" }),
    ]);
    expect(listing).toHaveBeenCalledTimes(1);
    expect(await new TaskStore(root).get(task.id)).toEqual(task);
    expect(
      JSON.parse(await readFile(join(root, ".motivo", "tasks", `${task.id}.json`), "utf8")),
    ).toEqual(task);
  });

  it("counts lead and investigators within the default four-call budget and reserves integration", async () => {
    const investigators = [deferred<ProviderResult>(), deferred<ProviderResult>()];
    let investigation = 0;
    const invoke = vi.fn(async (input: ProviderInvocation) => {
      if (input.prompt.includes("Your only assignment"))
        return investigators[investigation++]?.promise ?? returned(report());
      if (investigation === 0)
        return returned(
          report({
            action: "investigate",
            status: "continue",
            next: "Integrate findings",
            investigations: ["Question A", "Question B", "Question C"],
          }),
        );
      expect(input.prompt).toContain("Finding A");
      expect(input.prompt).toContain("Finding B");
      return returned(report({ summary: "Lead integrated both findings" }));
    });
    const { root, service, task } = await setup(invoke);
    await service.continue(
      root,
      taskContinueInputSchema.parse({ workspaceHandle, taskId: task.id }),
    );
    try {
      await waitUntil(() => invoke.mock.calls.length === 3);
      expect((await service.current(root, task.id)).status).toBe("running");
      // Even a completed report from an investigator cannot complete the task.
      investigators[1]?.resolve(returned(report({ summary: "Finding B" })));
      await waitUntil(async () =>
        (await service.current(root, task.id)).rounds.some(
          (round) => round.report?.summary === "Finding B",
        ),
      );
      expect((await service.current(root, task.id)).status).toBe("running");
      investigators[0]?.resolve(returned(report({ summary: "Finding A" })));
      await service.waitForIdle();
      const finished = await service.current(root, task.id);
      expect(finished.status).toBe("completed");
      expect(finished.calls).toBe(4);
      expect(finished.rounds.map((round) => round.role)).toEqual([
        "lead",
        "investigator",
        "investigator",
        "lead",
      ]);
      expect(
        invoke.mock.calls.some(([input]) =>
          input.prompt.includes("independent investigation: Question C"),
        ),
      ).toBe(false);
      expect(finished.message).toBe("Lead integrated both findings");
    } finally {
      investigators.forEach((pending) => pending.resolve(returned(report())));
    }
  });

  it("pauses when the call budget runs out with each handoff retained", async () => {
    let calls = 0;
    const invoke = vi.fn(async () =>
      returned(
        report({
          status: "continue",
          summary: `Observation ${++calls}`,
          next: `Work item ${calls}`,
        }),
      ),
    );
    const { root, service, task } = await setup(invoke);
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    const paused = await service.current(root, task.id);
    expect(paused.status).toBe("paused");
    expect(paused.calls).toBe(4);
    expect(paused.rounds.at(-1)?.report?.next).toBe("Work item 4");
    expect(invoke).toHaveBeenCalledTimes(4);
  });

  it("pauses identical handoffs but treats newly reported checks as progress", async () => {
    for (const newEvidence of [false, true]) {
      let calls = 0;
      const invoke = vi.fn(async () => {
        calls += 1;
        return returned(
          report({
            status: "continue",
            checks: [
              {
                name: "Focused check",
                result: "passed",
                detail: newEvidence ? `Checked case ${calls}` : "Same case",
              },
            ],
          }),
        );
      });
      const { root, service, task } = await setup(invoke);
      await service.continue(root, proceed(task.id));
      await service.waitForIdle();
      expect((await service.current(root, task.id)).status).toBe("paused");
      expect(invoke).toHaveBeenCalledTimes(newEvidence ? 4 : 2);
    }
  });

  it("pauses after the in-flight action without starting its proposed investigations", async () => {
    const pending = deferred<ProviderResult>();
    const invoke = vi.fn(() => pending.promise);
    const { root, service, task } = await setup(invoke);
    await service.continue(root, proceed(task.id));
    try {
      await waitUntil(() => invoke.mock.calls.length === 1);
      await service.pause(root, task.id);
      expect(service.busy).toBe(true);
      pending.resolve(
        returned(
          report({
            action: "investigate",
            status: "continue",
            next: "Read the tests",
            investigations: ["Check the parser"],
          }),
        ),
      );
      await service.waitForIdle();
      expect(await service.current(root, task.id)).toMatchObject({
        status: "paused",
        calls: 1,
        pauseRequested: false,
      });
      expect(invoke).toHaveBeenCalledTimes(1);
    } finally {
      pending.resolve(returned(report()));
    }
  });

  it("waits for active investigations on pause and saves both results", async () => {
    const pending = deferred<ProviderResult>();
    const invoke = vi.fn(async (input: ProviderInvocation) =>
      input.prompt.includes("Your only assignment")
        ? pending.promise
        : returned(
            report({
              action: "investigate",
              status: "continue",
              investigations: ["First", "Second"],
            }),
          ),
    );
    const { root, service, task } = await setup(invoke);
    await service.continue(root, proceed(task.id));
    try {
      await waitUntil(() => invoke.mock.calls.length === 3);
      await service.pause(root, task.id);
      pending.resolve(
        returned(
          report({ action: "investigate", status: "continue", summary: "Investigation findings" }),
        ),
      );
      await service.waitForIdle();
      const paused = await service.current(root, task.id);
      expect(paused.status).toBe("paused");
      expect(paused.calls).toBe(3);
      expect(
        paused.rounds.filter((round) => round.report?.summary === "Investigation findings"),
      ).toHaveLength(2);
    } finally {
      pending.resolve(returned(report()));
    }
  });

  it("recovers saved running work as unknown without retry and requires a reconciliation note", async () => {
    const invoke = vi.fn(async () => returned(report()));
    const { root, service, task } = await setup(invoke);
    await new TaskStore(root).update(task.id, (current) => ({
      ...current,
      status: "running",
      calls: 1,
      rounds: [savedRound({ outcome: "running", report: undefined })],
    }));
    const recovered = await service.current(root, task.id);
    expect(recovered.status).toBe("outcome_unknown");
    expect(recovered.rounds[0]?.outcome).toBe("outcome_unknown");
    expect(invoke).not.toHaveBeenCalled();
    await expect(service.continue(root, proceed(task.id))).rejects.toMatchObject({
      detail: { code: "task_reconciliation_required" },
    });
    await service.continue(
      root,
      proceed(
        task.id,
        "Inspected the workspace and provider records; continue from the saved files.",
      ),
    );
    await service.waitForIdle();
    expect(await service.current(root, task.id)).toMatchObject({ status: "completed", calls: 2 });
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("does not recover a running task after continue has reserved ownership during the read", async () => {
    const invoke = vi.fn(async () => returned(report()));
    const { root, service, task } = await setup(invoke);
    await new TaskStore(root).update(task.id, (current) => ({
      ...current,
      status: "running",
      calls: 1,
      rounds: [savedRound({ outcome: "running", report: undefined })],
    }));
    let continuation: Promise<TaskDocument> | undefined;
    const update = TaskStore.prototype.update;
    vi.spyOn(TaskStore.prototype, "update").mockImplementationOnce(function (
      this: TaskStore,
      id,
      change,
    ) {
      return update.call(this, id, (current) => {
        // current() has read the old record, but continue() synchronously
        // reserves ownership before the queued recovery callback runs.
        continuation = service.continue(
          root,
          proceed(task.id, "Inspected the interrupted operation."),
        );
        return change(current);
      });
    });
    const recovered = await service.current(root, task.id);
    expect(recovered.status).toBe("running");
    expect(recovered.rounds[0]?.outcome).toBe("running");
    expect(continuation).toBeDefined();
    await continuation;
    await service.waitForIdle();
    expect((await service.current(root, task.id)).status).toBe("completed");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("does not let an old reconciled unknown override a later known failure", async () => {
    const { root, service, task } = await setup(async () => {
      throw new ProviderCallError("Unavailable provider", "failed");
    });
    await new TaskStore(root).update(task.id, (current) => ({
      ...current,
      status: "outcome_unknown",
      rounds: [savedRound({ outcome: "outcome_unknown", report: undefined })],
    }));
    await service.continue(root, proceed(task.id, "Reconciled the previous invocation."));
    await service.waitForIdle();
    expect((await service.current(root, task.id)).status).toBe("failed");
  });

  it("preserves malformed reports as unknown and never repeats the completed external action", async () => {
    const invoke = vi.fn(async () => ({
      text: "I already changed the files, but this is not JSON.",
    }));
    const { root, service, task } = await setup(invoke);
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    expect(await service.current(root, task.id)).toMatchObject({
      status: "outcome_unknown",
      calls: 1,
      rounds: [expect.objectContaining({ outcome: "outcome_unknown" })],
    });
    await expect(service.continue(root, proceed(task.id))).rejects.toMatchObject({
      detail: { code: "task_reconciliation_required" },
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    expect((await new TaskStore(root).get(task.id)).rounds[0]).toMatchObject({
      rawOutput: "I already changed the files, but this is not JSON.",
      rawOutputTruncated: false,
    });
  });

  it("retains bounded malformed output for diagnosis without adding it to the next prompt", async () => {
    const rawOutput = "RAW-OUTPUT-DIAGNOSTIC\0" + "x".repeat(9000);
    const { root, service, task } = await setup(async () => ({ text: rawOutput }));
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    const failed = await new TaskStore(root).get(task.id);
    expect(failed.rounds[0]?.rawOutput).toBe(rawOutput.slice(0, 8000));
    expect(failed.rounds[0]?.rawOutputTruncated).toBe(true);
    expect(taskPrompt(failed, DEFAULT_METHOD, 3)).not.toContain("RAW-OUTPUT-DIAGNOSTIC");
  });

  it("continues after an answered question and includes the answer and previous question", async () => {
    const invoke = vi.fn(async (input: ProviderInvocation) => {
      if (context(input.prompt).userNotes.length === 0)
        return returned(
          report({ status: "needs_input", question: "Should invalid input return status 2?" }),
        );
      expect(context(input.prompt).userNotes.at(-1)?.text).toBe("Yes, use status 2.");
      expect(context(input.prompt).handoff?.question).toContain("status 2");
      return returned(report());
    });
    const { root, service, task } = await setup(invoke);
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    expect((await service.current(root, task.id)).status).toBe("needs_input");
    await expect(service.continue(root, proceed(task.id))).rejects.toMatchObject({
      detail: { code: "task_invalid" },
    });
    await service.continue(root, proceed(task.id, "Yes, use status 2."));
    await service.waitForIdle();
    expect(await service.current(root, task.id)).toMatchObject({ status: "completed", calls: 2 });
  });

  it("uses METHOD.md as the method override while retaining the report interface", async () => {
    const invoke = vi.fn<(input: ProviderInvocation) => Promise<ProviderResult>>(async () =>
      returned(report()),
    );
    const { root, service, task } = await setup(invoke);
    expect(await loadMethod(root)).toBe(DEFAULT_METHOD);
    await writeFile(
      join(root, ".motivo", "METHOD.md"),
      "CUSTOM METHOD: inspect one failing case first.",
    );
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    const prompt = invoke.mock.calls[0]?.[0]?.prompt;
    expect(prompt).toContain("CUSTOM METHOD");
    expect(prompt).not.toContain(DEFAULT_METHOD);
    expect(prompt).toContain("Return ONLY a JSON object");
  });

  it("accepts a valid long completion summary without losing its saved report", async () => {
    const summary = "a".repeat(8000);
    const { root, service, task } = await setup(async () => returned(report({ summary })));
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    const finished = await service.current(root, task.id);
    expect(finished.status).toBe("completed");
    expect(finished.message).toHaveLength(4000);
    expect(finished.rounds[0]?.report?.summary).toBe(summary);
  });

  it("does not rerun work when saving its result fails and recovers the earlier running record", async () => {
    let directory = "";
    const invoke = vi.fn(async () => {
      await rename(directory, `${directory}.saved`);
      await writeFile(directory, "simulate an unavailable task directory");
      return returned(report());
    });
    const { root, service, task } = await setup(invoke);
    directory = join(root, ".motivo", "tasks");
    await service.continue(root, proceed(task.id));
    await service.waitForIdle();
    expect(service.busy).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(1);
    await rm(directory);
    await rename(`${directory}.saved`, directory);
    expect((await service.current(root, task.id)).status).toBe("outcome_unknown");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("keeps the active task reserved after one investigation save fails until its sibling finishes", async () => {
    const sibling = deferred<ProviderResult>();
    const failedSave = deferred<undefined>();
    const original = TaskStore.prototype.update;
    vi.spyOn(TaskStore.prototype, "update").mockImplementation(function (
      this: TaskStore,
      id,
      change,
    ) {
      return original.call(this, id, (current) => {
        const next = change(current);
        if (next.rounds.some((round) => round.report?.summary === "Cannot save this branch")) {
          failedSave.resolve(undefined);
          throw new Error("Simulated disk failure");
        }
        return next;
      });
    });
    const invoke = vi.fn(async (input: ProviderInvocation) => {
      if (input.prompt.includes("independent investigation: First"))
        return returned(report({ summary: "Cannot save this branch" }));
      if (input.prompt.includes("independent investigation: Second")) return sibling.promise;
      return returned(
        report({ action: "investigate", status: "continue", investigations: ["First", "Second"] }),
      );
    });
    const { root, service, task } = await setup(invoke);
    await service.continue(root, proceed(task.id));
    try {
      await failedSave.promise;
      expect(service.busy).toBe(true);
      await expect(service.continue(root, proceed(task.id, "Try again"))).rejects.toMatchObject({
        detail: { code: "task_busy" },
      });
      sibling.resolve(returned(report({ summary: "Second branch finished" })));
      await service.waitForIdle();
      const failed = await service.current(root, task.id);
      expect(failed.status).toBe("outcome_unknown");
      expect(failed.rounds.some((round) => round.outcome === "running")).toBe(false);
      expect(invoke).toHaveBeenCalledTimes(3);
    } finally {
      sibling.resolve(returned(report()));
    }
  });
});

describe("task storage and prompt context", () => {
  it("serializes concurrent updates without dropping calls or rounds", async () => {
    const { root, task } = await setup(async () => returned(report()));
    const store = new TaskStore(root);
    const rounds = Array.from({ length: 12 }, () => savedRound());
    await Promise.all(
      rounds.map((round) =>
        store.update(task.id, (current) => ({
          ...current,
          calls: current.calls + 1,
          rounds: [...current.rounds, round],
        })),
      ),
    );
    const stored = await store.get(task.id);
    expect(stored.calls).toBe(12);
    expect(stored.revision).toBe(12);
    expect(stored.rounds.map((round) => round.id)).toEqual(rounds.map((round) => round.id));
  });

  it("bounds large histories by escaped request bytes and retains the latest lead handoff", async () => {
    const { task } = await setup(async () => returned(report()));
    const large = report({
      summary: "s".repeat(8000),
      findings: Array.from({ length: 30 }, () => ({ statement: "f".repeat(4000) })),
      checks: Array.from({ length: 30 }, () => ({
        name: "check",
        result: "unknown",
        detail: "c".repeat(4000),
      })),
      unknowns: Array.from({ length: 20 }, () => "u".repeat(2000)),
      artifacts: Array.from({ length: 30 }, () => "a".repeat(2000)),
      next: "PRESERVE THIS NEXT ACTION",
      decision: "PRESERVE THIS DECISION",
      status: "continue",
    });
    const document: TaskDocument = {
      ...task,
      rounds: Array.from({ length: 8 }, () => savedRound({ report: large })),
    };
    const prompt = taskPrompt(document, DEFAULT_METHOD, 3);
    expect(Buffer.byteLength(JSON.stringify({ prompt }))).toBeLessThanOrEqual(900 * 1024);
    const data = context(prompt);
    expect(data.omittedEarlierRounds).toBeGreaterThan(0);
    expect(data.handoff).toMatchObject({ next: large.next, decision: large.decision });
    expect(data.historySource).toBe(`.motivo/tasks/${task.id}.json`);
  });

  it("keeps the lead handoff when the latest eight reports are investigator findings", async () => {
    const { task } = await setup(async () => returned(report()));
    const document = {
      ...task,
      rounds: [
        savedRound({
          report: report({ next: "Integrate the parser decision", status: "continue" }),
        }),
        ...Array.from({ length: 9 }, () => savedRound({ role: "investigator" })),
      ],
    };
    const data = context(taskPrompt(document, DEFAULT_METHOD, 2));
    expect(data.recentHistory).toHaveLength(8);
    expect(data.handoff?.next).toBe("Integrate the parser decision");
    expect(data.omittedEarlierRounds).toBe(2);
  });

  it("accounts for nested JSON escaping while preserving the latest user answer", async () => {
    const { task } = await setup(async () => returned(report()));
    const escaped = "\u0001";
    const answer = "LATEST ANSWER" + escaped.repeat(7980);
    const document: TaskDocument = {
      ...task,
      goal: escaped.repeat(16000),
      constraints: escaped.repeat(8000),
      notes: Array.from({ length: 10 }, (_, index) => ({
        text: index === 9 ? answer : escaped.repeat(8000),
        at: task.createdAt,
      })),
      rounds: [
        savedRound({
          report: report({
            summary: escaped.repeat(8000),
            decision: escaped.repeat(4000),
            next: escaped.repeat(4000),
          }),
        }),
      ],
    };
    const prompt = taskPrompt(document, escaped.repeat(32768), 3);
    expect(Buffer.byteLength(JSON.stringify({ prompt }))).toBeLessThanOrEqual(900 * 1024);
    expect(context(prompt).userNotes.at(-1)?.text).toBe(answer);
    expect(context(prompt).omittedEarlierNotes).toBeGreaterThan(0);
  });

  it("accepts a fenced JSON task report but rejects missing required questions", () => {
    expect(parseTaskReport("```json\n" + JSON.stringify(report()) + "\n```")).toEqual(report());
    expect(() => parseTaskReport(JSON.stringify(report({ status: "needs_input" })))).toThrow();
  });
});

async function waitUntil(predicate: () => boolean | Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 2000;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error("Task fixture did not reach the expected state.");
}
