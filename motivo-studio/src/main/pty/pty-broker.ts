import { randomUUID } from "node:crypto";
import { utilityProcess, type UtilityProcess } from "electron";
import {
  terminalIdSchema,
  terminalProfileSchema,
  terminalSessionSchema,
  type TerminalId,
  type TerminalProfile,
  type TerminalSession,
  type Sequence,
} from "../../shared/contracts";
import { ptyHostEventSchema, type PtyCommand, type PtyEvent } from "../../shared/pty-protocol";
import { resolveShellProfiles } from "../../pty/shell-profiles";
import { StudioFault } from "../errors";

const CREATE_TIMEOUT_MS = 5_000;
const SHUTDOWN_TIMEOUT_MS = 2_000;
const MAX_SESSIONS = 8;

export interface PtyBroker {
  profiles(): readonly TerminalProfile[];
  create(input: {
    readonly cwd: string;
    readonly profileId: "powershell" | "bash";
    readonly cols: number;
    readonly rows: number;
  }): Promise<TerminalSession>;
  write(terminalId: TerminalId, data: string): void;
  resize(terminalId: TerminalId, cols: number, rows: number): void;
  acknowledge(terminalId: TerminalId, sequence: Sequence): void;
  close(terminalId: TerminalId): void;
  onEvent(listener: (event: PtyEvent) => void): () => void;
  shutdown(): Promise<void>;
}

interface PendingCreate {
  readonly terminalId: TerminalId;
  readonly profileId: "powershell" | "bash";
  readonly resolve: (session: TerminalSession) => void;
  readonly reject: (error: StudioFault) => void;
  readonly timer: NodeJS.Timeout;
}

export class UtilityProcessPtyBroker implements PtyBroker {
  private readonly child: UtilityProcess;
  private readonly listeners = new Set<(event: PtyEvent) => void>();
  private readonly pending = new Map<string, PendingCreate>();
  private readonly sessions = new Set<TerminalId>();
  private readonly handleChildMessage = (message: unknown): void => this.handleEvent(message);
  private readonly handleChildExit = (): void => this.handleExit();
  private stopped = false;
  private childExited = false;
  private shutdownStarted: Promise<void> | undefined;
  private resolveShutdown: (() => void) | undefined;
  private shutdownTimer: NodeJS.Timeout | undefined;

  constructor(modulePath: string, nodePtyRoot: string) {
    this.child = utilityProcess.fork(modulePath, [nodePtyRoot], {
      serviceName: "motivo-pty-broker",
      stdio: "ignore",
    });
    this.child.on("message", this.handleChildMessage);
    this.child.once("exit", this.handleChildExit);
  }

  profiles(): readonly TerminalProfile[] {
    return resolveShellProfiles().map((profile) =>
      terminalProfileSchema.parse({
        id: profile.id,
        label: profile.label,
        available: profile.available,
      }),
    );
  }

  create(input: {
    readonly cwd: string;
    readonly profileId: "powershell" | "bash";
    readonly cols: number;
    readonly rows: number;
  }): Promise<TerminalSession> {
    if (this.stopped) {
      return Promise.reject(ptyFault("PTY_BROKER_STOPPED", "The PTY broker is unavailable."));
    }
    if (this.sessions.size + this.pending.size >= MAX_SESSIONS) {
      return Promise.reject(ptyFault("PTY_SESSION_LIMIT", "The PTY session limit was reached."));
    }
    const commandId = randomUUID();
    const terminalId = terminalIdSchema.parse(randomUUID());
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (!this.pending.delete(commandId)) return;
        // A delayed create may still complete in the utility process. Queue a
        // matching close so that a timed-out request cannot orphan its PTY.
        this.post({ kind: "close", terminalId });
        reject(ptyFault("PTY_START_TIMEOUT", "The terminal did not start before its deadline."));
      }, CREATE_TIMEOUT_MS);
      this.pending.set(commandId, {
        terminalId,
        profileId: input.profileId,
        resolve,
        reject,
        timer,
      });
      this.post({ kind: "create", commandId, terminalId, ...input });
    });
  }

  write(terminalId: TerminalId, data: string): void {
    if (this.sessions.has(terminalId)) this.post({ kind: "write", terminalId, data });
  }

  resize(terminalId: TerminalId, cols: number, rows: number): void {
    if (this.sessions.has(terminalId)) this.post({ kind: "resize", terminalId, cols, rows });
  }

  acknowledge(terminalId: TerminalId, sequence: Sequence): void {
    if (this.sessions.has(terminalId)) this.post({ kind: "ack", terminalId, sequence });
  }

  close(terminalId: TerminalId): void {
    if (this.sessions.delete(terminalId)) this.post({ kind: "close", terminalId });
  }

  onEvent(listener: (event: PtyEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  shutdown(): Promise<void> {
    if (this.shutdownStarted) return this.shutdownStarted;
    this.stopped = true;
    this.rejectPending("PTY_BROKER_STOPPED", "The PTY broker stopped.");
    if (this.childExited) {
      this.shutdownStarted = Promise.resolve();
      return this.shutdownStarted;
    }
    this.shutdownStarted = new Promise<void>((resolve) => {
      this.resolveShutdown = resolve;
    });
    this.shutdownTimer = setTimeout(() => this.completeShutdown(true), SHUTDOWN_TIMEOUT_MS);
    if (!this.post({ kind: "shutdown" })) this.completeShutdown(true);
    return this.shutdownStarted;
  }

  private post(command: PtyCommand): boolean {
    if (!this.stopped || command.kind === "shutdown") {
      try {
        this.child.postMessage(command);
        return true;
      } catch {
        if (!this.stopped) this.handleExit();
      }
    }
    return false;
  }

  private handleEvent(raw: unknown): void {
    const parsed = ptyHostEventSchema.safeParse(raw);
    if (!parsed.success) return;
    const event = parsed.data;
    if (event.kind === "stopped") {
      if (this.shutdownStarted) this.completeShutdown(false);
      return;
    }
    if (event.kind === "created") {
      const pending = this.pending.get(event.commandId);
      if (!pending || pending.terminalId !== event.terminalId) {
        this.post({ kind: "close", terminalId: event.terminalId });
        return;
      }
      clearTimeout(pending.timer);
      this.pending.delete(event.commandId);
      this.sessions.add(event.terminalId);
      pending.resolve(
        terminalSessionSchema.parse({ id: event.terminalId, profileId: pending.profileId }),
      );
      return;
    }
    if (event.kind === "error" && event.commandId) {
      const pending = this.pending.get(event.commandId);
      if (pending) {
        clearTimeout(pending.timer);
        this.pending.delete(event.commandId);
        pending.reject(ptyFault(event.code, event.message));
      }
      return;
    }
    if (event.kind === "exit") this.sessions.delete(event.terminalId);
    this.listeners.forEach((listener) => listener(event));
  }

  private handleExit(): void {
    if (this.childExited) return;
    this.childExited = true;
    this.stopped = true;
    this.rejectPending("PTY_BROKER_STOPPED", "The PTY broker exited unexpectedly.");
    for (const terminalId of this.sessions) {
      const event: PtyEvent = {
        kind: "exit",
        terminalId,
        exitCode: null,
        reason: "broker-stopped",
      };
      this.listeners.forEach((listener) => listener(event));
    }
    this.sessions.clear();
    if (this.shutdownStarted) {
      this.completeShutdown(false);
      return;
    }
    this.listeners.clear();
    this.detachChildListeners();
  }

  private rejectPending(code: string, message: string): void {
    this.pending.forEach((pending) => {
      clearTimeout(pending.timer);
      pending.reject(ptyFault(code, message));
    });
    this.pending.clear();
  }

  private completeShutdown(kill: boolean): void {
    const resolve = this.resolveShutdown;
    if (!resolve) return;
    if (this.shutdownTimer) clearTimeout(this.shutdownTimer);
    this.shutdownTimer = undefined;
    this.resolveShutdown = undefined;
    this.detachChildListeners();
    if (kill && !this.childExited) {
      try {
        this.child.kill();
      } catch {
        // The utility may have exited between the deadline and the fallback.
      }
    }
    this.sessions.clear();
    this.listeners.clear();
    resolve();
  }

  private detachChildListeners(): void {
    this.child.off("message", this.handleChildMessage);
    this.child.off("exit", this.handleChildExit);
  }
}

function ptyFault(code: string, message: string): StudioFault {
  return new StudioFault({
    code,
    category: "resource",
    retryable: code !== "PTY_PROFILE_UNAVAILABLE",
    message,
  });
}
