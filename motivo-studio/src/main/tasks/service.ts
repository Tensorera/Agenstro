import { randomUUID } from "node:crypto";
import {
  taskCreateInputSchema,
  taskContinueInputSchema,
  type TaskCreateInput,
  type TaskContinueInput,
  type TaskDocument,
  type TaskRound,
  type TaskSummary,
} from "../../shared/task-contracts";
import { loadMethod, parseTaskReport, taskPrompt } from "./method";
import { TaskStore, taskError } from "./store";
import {
  invokeProvider,
  ProviderCallError,
  type ProviderInvocation,
  type ProviderResult,
} from "./transport";

interface ActiveTask {
  root: string;
  id: string;
  abort: AbortController;
  done?: Promise<void>;
}

export interface TaskServiceOptions {
  invoke?: (input: ProviderInvocation) => Promise<ProviderResult>;
}

/** A replaceable Motivo method loop. Every external agent action is supervised by Tactus. */
export class TaskService {
  private readonly stores = new Map<string, TaskStore>();
  private readonly invoke: (input: ProviderInvocation) => Promise<ProviderResult>;
  private active: ActiveTask | undefined;
  private disposed = false;

  constructor(options: TaskServiceOptions = {}) {
    this.invoke = options.invoke ?? invokeProvider;
  }

  get busy(): boolean {
    return this.active !== undefined;
  }

  private store(root: string): TaskStore {
    let store = this.stores.get(root);
    if (!store) {
      store = new TaskStore(root);
      this.stores.set(root, store);
    }
    return store;
  }

  async list(root: string): Promise<TaskSummary[]> {
    const summaries = await this.store(root).list();
    let recovered = false;
    for (const summary of summaries) {
      if (summary.status === "running" && !this.owns(root, summary.id)) {
        const task = await this.current(root, summary.id);
        recovered = recovered || task.status === "outcome_unknown";
      }
    }
    return recovered ? this.store(root).list() : summaries;
  }

  async current(root: string, id: string): Promise<TaskDocument> {
    const store = this.store(root);
    const task = await store.get(id);
    if (task.status !== "running" || this.owns(root, id)) return task;
    return store.update(id, (current) =>
      current.status !== "running" || this.owns(root, id)
        ? current
        : {
            ...current,
            status: "outcome_unknown",
            pauseRequested: false,
            message:
              "The previous process stopped during an action. Inspect the workspace and external effects before continuing.",
            rounds: current.rounds.map((round) =>
              round.outcome === "running"
                ? {
                    ...round,
                    outcome: "outcome_unknown",
                    finishedAt: new Date().toISOString(),
                    error: "Execution was interrupted; no task report was committed.",
                  }
                : round,
            ),
          },
    );
  }

  async create(root: string, raw: TaskCreateInput): Promise<TaskDocument> {
    if (this.disposed || this.busy)
      throw taskError("Wait for the current task before creating another.");
    const input = taskCreateInputSchema.parse(raw);
    const now = new Date().toISOString();
    return this.store(root).create({
      api: "motivo.task/v1",
      id: randomUUID(),
      goal: input.goal,
      constraints: input.constraints,
      provider: input.provider,
      status: "ready",
      createdAt: now,
      updatedAt: now,
      revision: 0,
      calls: 0,
      pauseRequested: false,
      notes: [],
      rounds: [],
    });
  }

  async continue(root: string, raw: TaskContinueInput): Promise<TaskDocument> {
    if (this.disposed || this.busy) throw taskError("A task is already running.", "task_busy");
    const input = taskContinueInputSchema.parse(raw);
    // Reserve synchronously, before I/O, so double clicks cannot start duplicate actions.
    const active: ActiveTask = { root, id: input.taskId, abort: new AbortController() };
    this.active = active;
    try {
      const store = this.store(root);
      const before = await store.get(input.taskId);
      if ((before.status === "outcome_unknown" || before.status === "running") && !input.note) {
        throw taskError(
          "Describe what you checked in the workspace or external system before continuing this interrupted task.",
          "task_reconciliation_required",
        );
      }
      if ((before.status === "completed" || before.status === "needs_input") && !input.note) {
        throw taskError("Add your answer or follow-up before continuing this task.");
      }
      if (before.rounds.length >= 196 || before.notes.length >= 100) {
        throw taskError(
          "This task reached its history limit. Start a new task with a concise handoff.",
        );
      }
      const method = await loadMethod(root);
      const started = await store.update(input.taskId, (task) => ({
        ...task,
        status: "running",
        pauseRequested: false,
        message: undefined,
        notes: input.note
          ? [...task.notes, { text: input.note, at: new Date().toISOString() }]
          : task.notes,
        rounds: task.rounds.map((round) =>
          round.outcome === "running"
            ? {
                ...round,
                outcome: "outcome_unknown",
                error: "Interrupted before a report was saved.",
              }
            : round,
        ),
      }));
      if (this.disposed) active.abort.abort();
      active.done = this.run(active, method, input.maxCalls)
        .catch(async () => {
          // Keep the saved running record if persistence itself is unavailable.
          // Opening it later conservatively recovers it as outcome_unknown.
          await store
            .update(active.id, (task) => ({
              ...task,
              status: "outcome_unknown",
              pauseRequested: false,
              message:
                "The task stopped before its result could be saved. Inspect the workspace before continuing.",
              rounds: task.rounds.map((round) =>
                round.outcome === "running"
                  ? {
                      ...round,
                      outcome: "outcome_unknown",
                      finishedAt: new Date().toISOString(),
                      error: "The action ended before its report could be saved.",
                    }
                  : round,
              ),
            }))
            .catch(() => undefined);
        })
        .finally(() => {
          if (this.active === active) this.active = undefined;
        });
      return started;
    } catch (error) {
      if (this.active === active) this.active = undefined;
      throw error;
    }
  }

  async pause(root: string, id: string): Promise<TaskDocument> {
    if (!this.owns(root, id)) return this.current(root, id);
    return this.store(root).update(id, (task) =>
      task.status === "running"
        ? {
            ...task,
            pauseRequested: true,
            message: "Pausing after the current action and any active investigations finish.",
          }
        : task,
    );
  }

  dispose(): void {
    this.disposed = true;
    this.active?.abort.abort();
  }

  /** Also used by model-free acceptance tests; no timer or polling loop needed. */
  async waitForIdle(): Promise<void> {
    await this.active?.done;
  }

  private owns(root: string, id: string): boolean {
    return this.active?.root === root && this.active.id === id;
  }

  private async run(active: ActiveTask, method: string, budget: number): Promise<void> {
    const store = this.store(active.root);
    let used = 0;
    let unproductive = 0;
    let previousSignature: string | undefined;
    while (used < budget && !active.abort.signal.aborted) {
      const task = await store.get(active.id);
      if (task.pauseRequested || task.rounds.length >= 196) break;
      used += 1;
      const lead = await this.perform(active, method, budget - used, "lead");
      if (lead.outcome !== "succeeded" || !lead.report) {
        await this.stopAfterFailure(active, [lead]);
        return;
      }
      const report = lead.report;
      if (report.status !== "continue") {
        await store.update(active.id, (current) => ({
          ...current,
          status: report.status === "completed" ? "completed" : "needs_input",
          pauseRequested: false,
          message: report.status === "completed" ? report.summary.slice(0, 4000) : report.question,
        }));
        return;
      }
      // Repeated identical handoffs are a useful observable stop signal, not a correctness judgment.
      const signature = JSON.stringify([
        report.summary,
        report.decision,
        report.next,
        report.findings,
        report.artifacts,
        report.checks,
        report.unknowns,
      ]);
      unproductive = signature === previousSignature ? unproductive + 1 : 0;
      previousSignature = signature;
      if (unproductive >= 1) {
        await store.update(active.id, (current) => ({
          ...current,
          status: "paused",
          pauseRequested: false,
          message:
            "Two actions returned the same handoff. Add guidance or choose a different approach before spending more calls.",
        }));
        return;
      }
      const latest = await store.get(active.id);
      if (latest.pauseRequested || active.abort.signal.aborted) break;
      // Reserve one call for the lead to integrate all returned findings.
      const questions = (report.investigations ?? []).slice(0, Math.max(0, budget - used - 1));
      if (questions.length > 0) {
        used += questions.length;
        const outcomes = await Promise.allSettled(
          questions.map((question) =>
            this.perform(active, method, budget - used, "investigator", question),
          ),
        );
        // A failed save must not release the active task while another branch
        // is still performing external work.
        for (const outcome of outcomes) {
          if (outcome.status === "rejected") throw outcome.reason;
        }
        const results = outcomes.flatMap((outcome) =>
          outcome.status === "fulfilled" ? [outcome.value] : [],
        );
        if (results.some((round) => round.outcome !== "succeeded")) {
          await this.stopAfterFailure(active, results);
          return;
        }
      }
    }
    await store.update(active.id, (task) => ({
      ...task,
      status: active.abort.signal.aborted ? "outcome_unknown" : "paused",
      pauseRequested: false,
      message: active.abort.signal.aborted
        ? "Execution was interrupted. Inspect the workspace before continuing."
        : "Paused with the current handoff saved. Continue with another call budget when useful.",
    }));
  }

  private async perform(
    active: ActiveTask,
    method: string,
    remaining: number,
    role: TaskRound["role"],
    question?: string,
  ): Promise<TaskRound> {
    const store = this.store(active.root);
    const task = await store.get(active.id);
    const round: TaskRound = {
      id: randomUUID(),
      role,
      focus:
        question ??
        (task.rounds.filter((entry) => entry.role === "lead").at(-1)?.report?.next ||
          task.goal.slice(0, 4000)),
      startedAt: new Date().toISOString(),
      outcome: "running",
    };
    await store.update(active.id, (current) => ({
      ...current,
      calls: current.calls + 1,
      rounds: [...current.rounds, round],
    }));
    let finished: TaskRound;
    let rawOutput: string | undefined;
    try {
      const result = await this.invoke({
        root: active.root,
        provider: task.provider,
        prompt: taskPrompt(task, method, remaining, question),
        signal: active.abort.signal,
      });
      rawOutput = result.text;
      const report = parseTaskReport(result.text);
      finished = { ...round, outcome: "succeeded", report };
    } catch (error) {
      finished = {
        ...round,
        outcome: error instanceof ProviderCallError ? error.outcome : "outcome_unknown",
        error:
          error instanceof Error
            ? error.message.slice(0, 4000)
            : "The task action did not return a usable result.",
        ...(rawOutput === undefined
          ? {}
          : {
              rawOutput: rawOutput.slice(0, 8000),
              rawOutputTruncated: rawOutput.length > 8000,
            }),
      };
    }
    finished.finishedAt = new Date().toISOString();
    finished.elapsedMs = Math.max(0, Date.now() - Date.parse(round.startedAt));
    await store.update(active.id, (current) => ({
      ...current,
      rounds: current.rounds.map((entry) => (entry.id === finished.id ? finished : entry)),
    }));
    return finished;
  }

  private async stopAfterFailure(active: ActiveTask, rounds: readonly TaskRound[]): Promise<void> {
    await this.store(active.root).update(active.id, (task) => {
      const unknown = rounds.some(
        (round) => round.outcome === "outcome_unknown" || round.outcome === "running",
      );
      return {
        ...task,
        status: unknown ? "outcome_unknown" : "failed",
        pauseRequested: false,
        message:
          "An action did not complete with a usable report. Review the recorded result and workspace before continuing.",
      };
    });
  }
}
