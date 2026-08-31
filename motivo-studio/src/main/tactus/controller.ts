import { spawn, type ChildProcessByStdio } from "node:child_process";
import { realpath } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import type { Readable } from "node:stream";
import { StringDecoder } from "node:string_decoder";
import { TextDecoder } from "node:util";
import { z } from "zod";
import {
  actionRequestSchema,
  decimalStringSchema,
  LIMITS,
  studioCheckSchema,
  studioEventSchema,
  studioActionEventSchema,
  studioEventPageSchema,
  studioOutcomeSchema,
  studioPluginSchema,
  studioRunSchema,
  studioScriptSchema,
  studioSnapshotSchema,
  studioSummarySchema,
  studioPresentationSchema,
  type ActionRequest,
  type ActionState,
  type StudioActionEvent,
  type StudioEventPage,
  type StudioSnapshot,
  type StudioPresentation,
  type StudioView,
} from "../../shared/contracts";
import {
  answeredAxisSchema,
  sessionAnswerInputSchema,
  sessionBriefSchema,
  sessionConsequenceSchema,
  sessionCurrentInputSchema,
  sessionFindingSchema,
  sessionListInputSchema,
  sessionListSchema,
  sessionOptionSchema,
  sessionQuestionSchema,
  sessionViewSchema,
  SESSION_LIMITS,
  type SessionAnswerInput,
  type SessionCurrentInput,
  type SessionList,
  type SessionListInput,
  type SessionView,
} from "../../shared/session-contracts";
import { MainProcessError } from "../errors";

const CONTROL_STDOUT_BYTES = 9 * 1_024 * 1_024;
const CONTROL_STDERR_BYTES = 64 * 1_024;
const CONTROL_MAX_CHUNKS = 4_096;
const INSPECT_TIMEOUT_MS = 30_000;
const INITIALIZE_TIMEOUT_MS = 120_000;
const EVENT_MAX_BYTES = 4 * 1_024 * 1_024;
const ACTION_MAX_BYTES = 16 * 1_024 * 1_024;
const ACTION_MAX_FRAMES = 4_096;
const ACTION_MAX_PENDING_FRAMES = 128;
const ACTION_FLUSH_INTERVAL_MS = 40;
const ACTION_FLUSH_BATCH = 32;
const CAPTURE_KILL_GRACE_MS = 2_000;
const ACTION_PROJECTION_WARNING =
  "Additional Tactus output was omitted after Motivo reached its bounded display limit.";

const inspectFailureSchema = z
  .object({
    api: z.literal("tactus.control/v1"),
    command: z.literal("studio.inspect"),
    status: z.literal("error"),
    error: z
      .object({
        code: z.string().min(1).max(96),
        message: z.string().refine((value) => [...value].length <= 4_096),
      })
      .strip(),
  })
  .passthrough();

const eventsFailureSchema = inspectFailureSchema.extend({
  command: z.literal("studio.events"),
});

const externalOutcomeSchema = studioOutcomeSchema.strip();
const externalRunSchema = studioRunSchema
  .extend({ outcome: externalOutcomeSchema.optional() })
  .strip();
const externalSnapshotSchema = studioSnapshotSchema
  .extend({
    workspace: z
      .object({
        name: z
          .string()
          .min(1)
          .refine((value) => [...value].length <= LIMITS.labelCharacters),
      })
      .strip(),
    health: z
      .object({ ok: z.boolean(), checks: z.array(studioCheckSchema.strip()).max(256) })
      .strip(),
    scripts: z.array(studioScriptSchema.strip()).max(10_000),
    registries: z
      .object({
        defaultProvider: z.string(),
        providers: z.array(studioPluginSchema.strip()).max(1_000),
        effects: z.array(studioPluginSchema.strip()).max(1_000),
        plugins: z.array(studioPluginSchema.strip()).max(1_000),
      })
      .strip(),
    runs: z.array(externalRunSchema).max(200),
  })
  .strip();
const externalSummarySchema = studioSummarySchema
  .extend({ outcome: externalOutcomeSchema })
  .strip();
const externalEventPageSchema = studioEventPageSchema
  .extend({
    run: externalRunSchema,
    events: z.array(studioEventSchema.strip()).max(1_000),
    summary: externalSummarySchema.optional(),
  })
  .strip();

const inspectControlSchema = z.discriminatedUnion("status", [
  z
    .object({
      api: z.literal("tactus.control/v1"),
      command: z.literal("studio.inspect"),
      status: z.literal("completed"),
      data: externalSnapshotSchema,
    })
    .passthrough(),
  inspectFailureSchema,
]);

const eventsControlSchema = z.discriminatedUnion("status", [
  z
    .object({
      api: z.literal("tactus.control/v1"),
      command: z.literal("studio.events"),
      status: z.literal("completed"),
      data: externalEventPageSchema,
    })
    .passthrough(),
  eventsFailureSchema,
]);

const externalSessionFindingSchema = sessionFindingSchema.strip();
const externalSessionOptionSchema = sessionOptionSchema.strip();
const externalSessionConsequenceSchema = sessionConsequenceSchema.strip();
const externalSessionQuestionSchema = sessionQuestionSchema
  .safeExtend({
    options: z.array(externalSessionOptionSchema).min(2).max(SESSION_LIMITS.optionCount),
  })
  .strip();
const externalSessionBriefSchema = sessionBriefSchema
  .safeExtend({
    findings: z.array(externalSessionFindingSchema).max(SESSION_LIMITS.briefFindings),
    question: externalSessionQuestionSchema,
    stakes: z.array(externalSessionConsequenceSchema).max(SESSION_LIMITS.optionCount),
  })
  .strip();
const externalSessionViewSchema = sessionViewSchema
  .safeExtend({
    pending: externalSessionBriefSchema.optional(),
    answered: z.array(answeredAxisSchema.strip()).max(SESSION_LIMITS.answeredAxes),
  })
  .strip();
const externalSessionListSchema = sessionListSchema
  .safeExtend({
    sessions: z.array(externalSessionViewSchema).max(SESSION_LIMITS.sessionPage),
  })
  .strip();

const sessionListFailureSchema = inspectFailureSchema.extend({
  command: z.literal("session.list"),
});
const sessionShowFailureSchema = inspectFailureSchema.extend({
  command: z.literal("session.show"),
});
const sessionAnswerFailureSchema = inspectFailureSchema.extend({
  command: z.literal("session.answer"),
});

const sessionListControlSchema = z.discriminatedUnion("status", [
  z
    .object({
      api: z.literal("tactus.control/v1"),
      command: z.literal("session.list"),
      status: z.literal("completed"),
      data: externalSessionListSchema,
    })
    .passthrough(),
  sessionListFailureSchema,
]);

const sessionShowControlSchema = z.discriminatedUnion("status", [
  z
    .object({
      api: z.literal("tactus.control/v1"),
      command: z.literal("session.show"),
      status: z.literal("completed"),
      data: externalSessionViewSchema,
    })
    .passthrough(),
  sessionShowFailureSchema,
]);

const sessionAnswerControlSchema = z.discriminatedUnion("status", [
  z
    .object({
      api: z.literal("tactus.control/v1"),
      command: z.literal("session.answer"),
      status: z.literal("completed"),
      data: externalSessionViewSchema,
    })
    .passthrough(),
  sessionAnswerFailureSchema,
]);

type TactusChild = ChildProcessByStdio<null, Readable, Readable>;

interface ActiveAction {
  readonly actionId: string;
  readonly kind: ActionRequest["kind"];
  readonly root: string;
  readonly child: TactusChild;
  sequence: bigint;
  cancelRequested: boolean;
  finished: boolean;
  outputBytes: number;
  outputFrames: number;
  pending: Array<{
    stream: "stdout" | "stderr";
    text: string;
    presentation?: StudioPresentation;
  }>;
  lineBuffers: Record<"stdout" | "stderr", string>;
  flushTimer: ReturnType<typeof setTimeout> | undefined;
  projectionDropped: boolean;
}

interface CaptureResult {
  readonly exitCode: number | null;
  readonly stdout: string;
  readonly stderr: string;
}

export interface TactusControllerOptions {
  readonly executable?: string;
  /** Fixed argv prefix used by tests and executable wrappers; never renderer-controlled. */
  readonly commandPrefix?: readonly string[];
  readonly actionOutputLimitBytes?: number;
  readonly actionOutputLimitFrames?: number;
  readonly actionPendingLimitFrames?: number;
  readonly emit: (event: StudioActionEvent) => void;
}

/** Main-process owner of the active workspace and every Tactus subprocess. */
export class TactusController {
  private readonly executable: string;
  private readonly commandPrefix: readonly string[];
  private readonly actionOutputLimitBytes: number;
  private readonly actionOutputLimitFrames: number;
  private readonly actionPendingLimitFrames: number;
  private readonly emitEvent: (event: StudioActionEvent) => void;
  private workspaceRoot: string | undefined;
  private workspaceHandle: string | undefined;
  private snapshot: StudioSnapshot | undefined;
  private active: ActiveAction | undefined;
  private readonly controlChildren = new Set<TactusChild>();
  private controlQueue: Promise<void> = Promise.resolve();
  private pendingControlOperations = 0;
  private readonly controlDrainWaiters = new Set<() => void>();
  private disposed = false;

  constructor(options: TactusControllerOptions) {
    this.executable =
      options.executable ?? process.env.MOTIVO_TACTUS_BIN ?? process.env.TACTUS_BIN ?? "tactus";
    this.commandPrefix = options.commandPrefix ?? [];
    this.actionOutputLimitBytes = options.actionOutputLimitBytes ?? ACTION_MAX_BYTES;
    this.actionOutputLimitFrames = options.actionOutputLimitFrames ?? ACTION_MAX_FRAMES;
    this.actionPendingLimitFrames = options.actionPendingLimitFrames ?? ACTION_MAX_PENDING_FRAMES;
    this.emitEvent = options.emit;
  }

  current(): StudioView | null {
    return this.view();
  }

  async open(root: string): Promise<StudioView> {
    return this.scheduleControl(async () => {
      const resolved = await realpath(root);
      const snapshot = await this.inspect(resolved);
      this.workspaceRoot = resolved;
      this.workspaceHandle = randomUUID();
      this.snapshot = snapshot;
      return this.requiredView();
    });
  }

  async initialize(root: string): Promise<StudioView> {
    return this.scheduleControl(async () => {
      const resolved = await realpath(root);
      const initialized = await this.capture(
        ["init", resolved, "--json"],
        resolved,
        INITIALIZE_TIMEOUT_MS,
      );
      if (initialized.exitCode !== 0) {
        throw this.processFailure("initialize_failed", initialized, resolved);
      }
      const snapshot = await this.inspect(resolved);
      this.workspaceRoot = resolved;
      this.workspaceHandle = randomUUID();
      this.snapshot = snapshot;
      return this.requiredView();
    });
  }

  async refresh(): Promise<StudioView> {
    return this.scheduleControl(async () => {
      const root = this.requireWorkspace();
      this.snapshot = await this.inspect(root);
      return this.requiredView();
    });
  }

  async events(
    runId: string,
    after: string,
    limit: number = LIMITS.eventPage,
  ): Promise<StudioEventPage> {
    const parsedAfter = decimalStringSchema.parse(after);
    return this.scheduleControl(async () => {
      const root = this.requireWorkspace();
      const result = await this.capture(
        [
          "studio",
          "events",
          runId,
          "--root",
          root,
          "--after",
          parsedAfter,
          "--limit",
          String(limit),
          "--max-bytes",
          String(EVENT_MAX_BYTES),
        ],
        root,
        INSPECT_TIMEOUT_MS,
      );
      const envelope = parseControl(eventsControlSchema, result, root);
      if (envelope.status === "error") throw controlFailure(envelope.error, root);
      return studioEventPageSchema.parse(redactValue(envelope.data, root));
    });
  }

  async sessions(rawInput: SessionListInput): Promise<SessionList> {
    const input = sessionListInputSchema.parse(rawInput);
    const limit = input.limit ?? 50;
    return this.scheduleControl(async () => {
      const root = this.requireWorkspaceHandle(input.workspaceHandle);
      const result = await this.capture(
        ["session", "list", "--root", root, "--limit", String(limit)],
        root,
        INSPECT_TIMEOUT_MS,
      );
      const envelope = parseControl(sessionListControlSchema, result, root);
      if (envelope.status === "error") throw sessionControlFailure(envelope.error, root);
      return sessionListSchema.parse(redactValue(envelope.data, root));
    });
  }

  async session(rawInput: SessionCurrentInput): Promise<SessionView> {
    const input = sessionCurrentInputSchema.parse(rawInput);
    return this.scheduleControl(async () => {
      const root = this.requireWorkspaceHandle(input.workspaceHandle);
      const result = await this.capture(
        ["session", "show", "--root", root, "--session", input.sessionId],
        root,
        INSPECT_TIMEOUT_MS,
      );
      const envelope = parseControl(sessionShowControlSchema, result, root);
      if (envelope.status === "error") throw sessionControlFailure(envelope.error, root);
      return sessionViewSchema.parse(redactValue(envelope.data, root));
    });
  }

  async answer(rawInput: SessionAnswerInput): Promise<SessionView> {
    const input = sessionAnswerInputSchema.parse(rawInput);
    return this.scheduleControl(async () => {
      const root = this.requireWorkspaceHandle(input.workspaceHandle);
      const result = await this.capture(
        [
          "session",
          "answer",
          "--root",
          root,
          "--session",
          input.sessionId,
          "--turn",
          input.turn,
          "--axis",
          input.axis,
          "--option",
          input.option,
          ...(input.note !== undefined ? ["--note", input.note] : []),
        ],
        root,
        INSPECT_TIMEOUT_MS,
      );
      const envelope = parseControl(sessionAnswerControlSchema, result, root);
      if (envelope.status === "error") throw sessionControlFailure(envelope.error, root);
      return sessionViewSchema.parse(redactValue(envelope.data, root));
    });
  }

  start(rawRequest: ActionRequest): ActionState {
    const root = this.requireWorkspace();
    this.requireIdle();
    this.requireControlIdle();
    const request = actionRequestSchema.parse(rawRequest);
    const actionId = randomUUID();
    const startedAtUnixMs = Date.now().toString();
    const child = this.spawn(commandForAction(request, root), root);
    const active: ActiveAction = {
      actionId,
      kind: request.kind,
      root,
      child,
      sequence: 0n,
      cancelRequested: false,
      finished: false,
      outputBytes: 0,
      outputFrames: 0,
      pending: [],
      lineBuffers: { stdout: "", stderr: "" },
      flushTimer: undefined,
      projectionDropped: false,
    };
    this.active = active;
    const state: ActionState = { actionId, kind: request.kind, startedAtUnixMs };
    this.emit({ type: "started", ...state });
    this.project(child.stdout, "stdout", active);
    this.project(child.stderr, "stderr", active);
    child.once("error", (error) => this.finish(active, null, error.message));
    child.once("close", (code) => this.finish(active, code, undefined));
    return state;
  }

  cancel(actionId: string): void {
    const active = this.active;
    if (!active || active.actionId !== actionId) {
      throw new MainProcessError({
        code: "action_not_running",
        category: "validation",
        retryable: false,
        message: "The selected action is no longer running.",
      });
    }
    active.cancelRequested = true;
    if (!active.child.kill()) {
      throw new MainProcessError({
        code: "cancel_failed",
        category: "process",
        retryable: true,
        message: "Tactus did not accept the cancellation request.",
      });
    }
  }

  dispose(): void {
    this.disposed = true;
    if (this.active && !this.active.finished) {
      this.active.cancelRequested = true;
      this.active.child.kill();
    }
    for (const child of this.controlChildren) child.kill();
    this.controlChildren.clear();
    this.resolveControlDrain();
  }

  private scheduleControl<Result>(operation: () => Promise<Result>): Promise<Result> {
    if (this.disposed) return Promise.reject(new Error("Tactus controller is disposed."));
    this.requireIdle();
    this.pendingControlOperations += 1;
    const scheduled = this.controlQueue.then(async () => {
      if (this.disposed) throw new Error("Tactus controller is disposed.");
      this.requireIdle();
      return operation();
    });
    this.controlQueue = scheduled.then(
      () => this.waitForControlDrain(),
      () => this.waitForControlDrain(),
    );
    return scheduled.finally(() => {
      this.pendingControlOperations -= 1;
    });
  }

  private waitForControlDrain(): Promise<void> {
    if (this.controlChildren.size === 0) return Promise.resolve();
    return new Promise((resolve) => this.controlDrainWaiters.add(resolve));
  }

  private resolveControlDrain(): void {
    if (this.controlChildren.size > 0) return;
    for (const resolve of this.controlDrainWaiters) resolve();
    this.controlDrainWaiters.clear();
  }

  private async inspect(root: string): Promise<StudioSnapshot> {
    const result = await this.capture(
      ["studio", "inspect", "--root", root, "--exact-root", "--run-limit", "50"],
      root,
      INSPECT_TIMEOUT_MS,
    );
    const envelope = parseControl(inspectControlSchema, result, root);
    if (envelope.status === "error") throw controlFailure(envelope.error, root);
    return studioSnapshotSchema.parse(redactValue(envelope.data, root));
  }

  private capture(args: readonly string[], cwd: string, timeoutMs: number): Promise<CaptureResult> {
    this.requireIdle();
    if (this.controlChildren.size > 0) {
      throw new Error("control scheduler invariant failed");
    }
    return new Promise((resolve, reject) => {
      const child = this.spawn(args, cwd);
      this.controlChildren.add(child);
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      let stdoutBytes = 0;
      let stderrBytes = 0;
      let settled = false;
      let overflow: "stdout" | "stderr" | undefined;
      let deadlineReached = false;
      let killGraceTimer: ReturnType<typeof setTimeout> | undefined;
      const timeoutFailure = () =>
        new MainProcessError({
          code: "tactus_timeout",
          category: "process",
          retryable: true,
          message: "Tactus control request exceeded its deadline.",
        });
      const overflowFailure = () =>
        new MainProcessError({
          code: "control_output_too_large",
          category: "process",
          retryable: false,
          message: `Tactus ${overflow ?? "output"} exceeded the control-plane byte budget.`,
        });
      const unavailableFailure = () =>
        new MainProcessError({
          code: "tactus_unavailable",
          category: "process",
          retryable: true,
          message: "Tactus could not be started. Check MOTIVO_TACTUS_BIN or PATH.",
        });
      const currentFailure = () => {
        if (deadlineReached) return timeoutFailure();
        if (overflow) return overflowFailure();
        return unavailableFailure();
      };
      const rejectOnce = (failure: MainProcessError): void => {
        if (settled) return;
        settled = true;
        reject(failure);
      };
      const cleanupStoppedChild = (): void => {
        this.controlChildren.delete(child);
        this.resolveControlDrain();
        clearTimeout(timer);
        if (killGraceTimer) clearTimeout(killGraceTimer);
      };
      const waitForStop = (): void => {
        killGraceTimer ??= setTimeout(() => {
          if (!this.controlChildren.has(child)) return;
          child.kill("SIGKILL");
          rejectOnce(currentFailure());
        }, CAPTURE_KILL_GRACE_MS);
      };
      const timer = setTimeout(() => {
        if (settled) return;
        deadlineReached = true;
        child.kill();
        waitForStop();
      }, timeoutMs);

      child.stdout.on("data", (chunk: Buffer) => {
        if (settled || overflow || deadlineReached) return;
        stdoutBytes += chunk.byteLength;
        if (stdoutBytes > CONTROL_STDOUT_BYTES || stdout.length >= CONTROL_MAX_CHUNKS) {
          overflow = "stdout";
          clearTimeout(timer);
          child.kill();
          waitForStop();
          return;
        }
        stdout.push(chunk);
      });
      child.stderr.on("data", (chunk: Buffer) => {
        if (settled || overflow || deadlineReached) return;
        stderrBytes += chunk.byteLength;
        if (stderrBytes > CONTROL_STDERR_BYTES || stderr.length >= CONTROL_MAX_CHUNKS) {
          overflow = "stderr";
          clearTimeout(timer);
          child.kill();
          waitForStop();
          return;
        }
        stderr.push(chunk);
      });
      child.once("error", () => {
        // A spawn failure has no live process and is therefore confirmed stopped.
        // Other child errors do not prove termination: retain the control lock
        // until `close`, or until controller disposal clears the process set.
        if (child.pid === undefined) {
          cleanupStoppedChild();
        } else if (!deadlineReached && !overflow) {
          child.kill();
          waitForStop();
        }
        rejectOnce(currentFailure());
      });
      child.once("close", (exitCode) => {
        // Promise settlement and child termination are deliberately separate.
        // A deadline may reject before this event, but only `close` releases the
        // control-plane lock for a process that actually started.
        cleanupStoppedChild();
        if (settled) return;
        if (deadlineReached) {
          rejectOnce(timeoutFailure());
          return;
        }
        if (overflow) {
          rejectOnce(overflowFailure());
          return;
        }
        let decodedStdout: string;
        try {
          decodedStdout = new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(stdout));
        } catch {
          rejectOnce(
            new MainProcessError({
              code: "invalid_control_utf8",
              category: "process",
              retryable: false,
              message: "Tactus control stdout was not valid UTF-8.",
            }),
          );
          return;
        }
        settled = true;
        resolve({
          exitCode,
          stdout: decodedStdout,
          stderr: Buffer.concat(stderr).toString("utf8"),
        });
      });
    });
  }

  private spawn(args: readonly string[], cwd: string): TactusChild {
    return spawn(this.executable, [...this.commandPrefix, ...args], {
      cwd,
      shell: false,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
  }

  private project(stream: Readable, streamName: "stdout" | "stderr", active: ActiveAction): void {
    const decoder = new StringDecoder("utf8");
    stream.on("data", (chunk: Buffer) => {
      this.enqueueOutput(active, streamName, decoder.write(chunk));
    });
    stream.once("end", () => {
      const tail = decoder.end();
      if (tail) this.enqueueOutput(active, streamName, tail);
      this.flushOutputTail(active, streamName);
    });
  }

  private enqueueOutput(active: ActiveAction, stream: "stdout" | "stderr", rawText: string): void {
    if (active.finished || active.projectionDropped || !rawText) return;
    const bytes = Buffer.byteLength(rawText, "utf8");
    if (active.outputBytes + bytes > this.actionOutputLimitBytes) {
      this.dropOutputProjection(active, stream);
      return;
    }
    active.outputBytes += bytes;
    const text = redactText(rawText, active.root);
    active.lineBuffers[stream] += text;
    let newline = active.lineBuffers[stream].indexOf("\n");
    while (newline >= 0) {
      const line = active.lineBuffers[stream].slice(0, newline + 1);
      active.lineBuffers[stream] = active.lineBuffers[stream].slice(newline + 1);
      this.queueOutput(active, stream, line);
      if (active.projectionDropped) return;
      newline = active.lineBuffers[stream].indexOf("\n");
    }
    this.scheduleFlush(active);
  }

  private flushOutputTail(active: ActiveAction, stream: "stdout" | "stderr"): void {
    const tail = active.lineBuffers[stream];
    active.lineBuffers[stream] = "";
    if (tail) this.queueOutput(active, stream, tail);
    this.scheduleFlush(active);
  }

  private queueOutput(active: ActiveAction, stream: "stdout" | "stderr", text: string): void {
    if (active.projectionDropped) return;
    const parts = splitUtf8(text, LIMITS.actionOutputBytes);
    for (const part of parts) {
      const presentation = parts.length === 1 ? parseTactusPresentation(part) : undefined;
      const previous = active.pending.at(-1);
      if (
        !presentation &&
        !previous?.presentation &&
        previous?.stream === stream &&
        Buffer.byteLength(previous.text, "utf8") + Buffer.byteLength(part, "utf8") <=
          LIMITS.actionOutputBytes
      ) {
        previous.text += part;
        continue;
      }
      if (active.pending.length >= this.actionPendingLimitFrames) {
        this.flushOutput(active, false);
      }
      if (
        active.outputFrames >= this.actionOutputLimitFrames ||
        active.pending.length >= this.actionPendingLimitFrames
      ) {
        this.dropOutputProjection(active, stream);
        return;
      }
      active.pending.push({
        stream,
        text: part,
        ...(presentation ? { presentation } : {}),
      });
      active.outputFrames += 1;
    }
  }

  private scheduleFlush(active: ActiveAction): void {
    if (active.flushTimer || active.finished || active.pending.length === 0) return;
    active.flushTimer = setTimeout(() => {
      active.flushTimer = undefined;
      this.flushOutput(active, false);
    }, ACTION_FLUSH_INTERVAL_MS);
  }

  private flushOutput(active: ActiveAction, all: boolean): void {
    if (active.flushTimer) {
      clearTimeout(active.flushTimer);
      active.flushTimer = undefined;
    }
    const count = all ? active.pending.length : Math.min(active.pending.length, ACTION_FLUSH_BATCH);
    const frames = active.pending.splice(0, count);
    for (const frame of frames) {
      active.sequence += 1n;
      this.emit({
        type: "output",
        actionId: active.actionId,
        sequence: active.sequence.toString(),
        stream: frame.stream,
        text: frame.text,
        ...(frame.presentation ? { presentation: frame.presentation } : {}),
      });
    }
    if (active.pending.length > 0) this.scheduleFlush(active);
  }

  private dropOutputProjection(active: ActiveAction, stream: "stdout" | "stderr"): void {
    if (active.projectionDropped || active.finished) return;
    active.projectionDropped = true;
    active.lineBuffers.stdout = "";
    active.lineBuffers.stderr = "";
    this.flushOutput(active, true);
    active.sequence += 1n;
    this.emit({
      type: "output",
      actionId: active.actionId,
      sequence: active.sequence.toString(),
      stream,
      text: `[warning] ${ACTION_PROJECTION_WARNING}\n`,
      presentation: {
        category: "warning",
        message: ACTION_PROJECTION_WARNING,
      },
    });
  }

  private finish(active: ActiveAction, exitCode: number | null, error: string | undefined): void {
    if (active.finished) return;
    this.flushOutputTail(active, "stdout");
    this.flushOutputTail(active, "stderr");
    this.flushOutput(active, true);
    active.finished = true;
    if (this.active === active) this.active = undefined;
    active.sequence += 1n;
    const status = active.cancelRequested ? "cancelled" : exitCode === 0 ? "succeeded" : "failed";
    this.emit({
      type: "finished",
      actionId: active.actionId,
      sequence: active.sequence.toString(),
      status,
      exitCode,
      finishedAtUnixMs: Date.now().toString(),
      ...(error ? { message: boundedDiagnostic(error, active.root) } : {}),
    });
  }

  private emit(event: StudioActionEvent): void {
    this.emitEvent(studioActionEventSchema.parse(event));
  }

  private requireWorkspace(): string {
    if (!this.workspaceRoot) {
      throw new MainProcessError({
        code: "workspace_not_open",
        category: "workspace",
        retryable: false,
        message: "Open an initialized Tactus workspace first.",
      });
    }
    return this.workspaceRoot;
  }

  private requireWorkspaceHandle(handle: string): string {
    const root = this.requireWorkspace();
    if (!this.workspaceHandle || handle !== this.workspaceHandle) {
      throw new MainProcessError({
        code: "workspace_handle_stale",
        category: "validation",
        retryable: false,
        message: "The workspace changed before this session request could start.",
      });
    }
    return root;
  }

  private requireIdle(): void {
    if (this.active) {
      throw new MainProcessError({
        code: "action_busy",
        category: "busy",
        retryable: true,
        message: "Wait for the active Tactus action to finish or cancel it.",
      });
    }
  }

  private requireControlIdle(): void {
    if (this.pendingControlOperations > 0 || this.controlChildren.size > 0) {
      throw new MainProcessError({
        code: "control_busy",
        category: "busy",
        retryable: true,
        message: "Wait for the current Studio query to finish.",
      });
    }
  }

  private view(): StudioView | null {
    if (!this.workspaceHandle || !this.snapshot) return null;
    return { handle: this.workspaceHandle, snapshot: this.snapshot };
  }

  private requiredView(): StudioView {
    const value = this.view();
    if (!value) throw new Error("workspace view invariant failed");
    return value;
  }

  private processFailure(code: string, result: CaptureResult, root: string): MainProcessError {
    return new MainProcessError({
      code,
      category: "process",
      retryable: true,
      message: boundedDiagnostic(result.stderr || result.stdout || "Tactus failed.", root),
    });
  }
}

/** Parse only Tactus's canonical human presentation line; all other text remains raw. */
export function parseTactusPresentation(value: string): StudioPresentation | undefined {
  const matched = /^\[(state|info|warning|error)\][\t ]+([^\r\n]+)(?:\r?\n)?$/.exec(value);
  if (!matched) return undefined;
  const parsed = studioPresentationSchema.safeParse({
    category: matched[1],
    message: matched[2]?.trim(),
  });
  return parsed.success ? parsed.data : undefined;
}

export function commandForAction(request: ActionRequest, root: string): string[] {
  switch (request.kind) {
    case "generate":
      return [
        "generate",
        "--root",
        root,
        ...(request.provider ? ["--provider", request.provider] : []),
        request.goal,
      ];
    case "check":
      return ["check", "--root", root];
    case "run":
      return ["run", "--root", root];
    case "smoke":
      return [
        "smoke",
        "--root",
        root,
        ...(request.live ? ["--live"] : []),
        ...request.targets.map((target) => `${target.namespace}:${target.name}`),
      ];
  }
}

function parseControl<Output>(
  schema: z.ZodType<Output>,
  result: CaptureResult,
  root: string,
): Output {
  let decoded: unknown;
  try {
    decoded = JSON.parse(result.stdout);
  } catch {
    throw new MainProcessError({
      code: "invalid_control_response",
      category: "process",
      retryable: true,
      message: boundedDiagnostic(result.stderr || "Tactus returned invalid control JSON.", root),
    });
  }
  const parsed = schema.safeParse(decoded);
  if (!parsed.success) {
    throw new MainProcessError({
      code: "invalid_control_response",
      category: "process",
      retryable: false,
      message: "Tactus returned a response that does not match tactus.control/v1.",
    });
  }
  if (result.exitCode !== 0 && (parsed.data as { status?: unknown }).status !== "error") {
    throw new MainProcessError({
      code: "tactus_failed",
      category: "process",
      retryable: true,
      message: boundedDiagnostic(result.stderr || "Tactus failed.", root),
    });
  }
  return parsed.data;
}

function controlFailure(error: { code: string; message: string }, root: string): MainProcessError {
  return new MainProcessError({
    code: normalizeErrorCode(error.code),
    category: "workspace",
    retryable: true,
    message: boundedDiagnostic(error.message, root),
  });
}

function sessionControlFailure(
  error: { code: string; message: string },
  root: string,
): MainProcessError {
  const code = normalizeErrorCode(error.code);
  const domainCode = code.startsWith("session_") ? code.slice("session_".length) : code;
  const validation = new Set([
    "invalid_argument",
    "invalid_id",
    "turn_stale",
    "axis_mismatch",
    "option_invalid",
    "state_invalid",
  ]).has(domainCode);
  const terminalWorkspace = new Set(["not_found", "corrupt"]).has(domainCode);
  return new MainProcessError({
    code,
    category: validation ? "validation" : "workspace",
    retryable: validation || terminalWorkspace ? false : true,
    message: boundedDiagnostic(error.message, root),
  });
}

function normalizeErrorCode(value: string): string {
  const normalized = value.toLowerCase().replaceAll(/[^a-z0-9_]/g, "_");
  return /^[a-z]/.test(normalized) ? normalized.slice(0, 96) : "tactus_control_failed";
}

function boundedDiagnostic(value: string, root: string): string {
  const redacted = redactText(value, root).trim();
  return [...(redacted || "Tactus failed without a diagnostic.")]
    .slice(0, LIMITS.diagnosticCharacters)
    .join("");
}

function redactText(value: string, root: string): string {
  if (!root) return value;
  const variants = new Set([root, root.replaceAll("\\", "/"), root.replaceAll("/", "\\")]);
  let redacted = value;
  for (const variant of variants) {
    const escaped = variant.replaceAll(/[.*+?^${}()|[\]\\]/g, "\\$&");
    redacted = redacted.replace(
      new RegExp(escaped, process.platform === "win32" ? "gi" : "g"),
      "<workspace>",
    );
  }
  return redacted;
}

function redactValue(value: unknown, root: string): unknown {
  if (typeof value === "string") return redactText(value, root);
  if (Array.isArray(value)) return value.map((entry) => redactValue(entry, root));
  if (value && typeof value === "object") {
    const entries: Array<[string, unknown]> = [];
    const redactedKeys = new Set<string>();
    for (const [key, entry] of Object.entries(value)) {
      const redactedKey = redactText(key, root);
      if (redactedKeys.has(redactedKey)) {
        throw new MainProcessError({
          code: "redaction_key_collision",
          category: "internal",
          retryable: false,
          message: "Tactus output could not be projected without exposing workspace data.",
        });
      }
      redactedKeys.add(redactedKey);
      entries.push([redactedKey, redactValue(entry, root)]);
    }
    return Object.fromEntries(entries);
  }
  return value;
}

export function splitUtf8(value: string, maximumBytes: number): string[] {
  if (!value) return [];
  const parts: string[] = [];
  let current = "";
  let bytes = 0;
  for (const scalar of value) {
    const scalarBytes = Buffer.byteLength(scalar, "utf8");
    if (bytes + scalarBytes > maximumBytes && current) {
      parts.push(current);
      current = "";
      bytes = 0;
    }
    current += scalar;
    bytes += scalarBytes;
  }
  if (current) parts.push(current);
  return parts;
}
