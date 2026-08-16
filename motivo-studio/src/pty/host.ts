import type { IPty } from "node-pty";
import { sequenceSchema, terminalIdSchema, utf8Bytes } from "../shared/contracts";
import { ptyCommandSchema, type PtyEvent, type PtyHostEvent } from "../shared/pty-protocol";
import { OutputWindow, splitUtf8 } from "./backpressure";
import { resolveShellProfiles, terminalEnvironment } from "./shell-profiles";
import type { PtySpawn } from "./node-pty-loader";

const MAX_SESSIONS = 8;

export interface PtyParentPort {
  postMessage(message: PtyHostEvent): void;
  on(event: "message", listener: (event: unknown) => void): void;
  off(event: "message", listener: (event: unknown) => void): void;
}

interface Session {
  readonly terminal: IPty;
  readonly output: OutputWindow;
  sequence: number;
}

export function runPtyHost(
  parentPort: PtyParentPort,
  spawnPty: PtySpawn,
  onShutdownAcknowledged?: () => void,
): () => void {
  const sessions = new Map<string, Session>();
  let stopped = false;

  const post = (event: PtyHostEvent) => parentPort.postMessage(event);
  const finish = (
    terminalId: string,
    exitCode: number | null,
    reason: "exited" | "closed" | "output-backpressure" | "broker-stopped",
  ) => {
    const session = sessions.get(terminalId);
    if (!session) return;
    sessions.delete(terminalId);
    if (reason !== "exited") {
      try {
        session.terminal.kill();
      } catch {
        // A concurrent native exit may have already reaped the PTY.
      }
    }
    const event: PtyEvent = {
      kind: "exit",
      terminalId: terminalIdSchema.parse(terminalId),
      exitCode,
      reason,
    };
    if (!stopped) {
      post(event);
      return;
    }
    try {
      post(event);
    } catch {
      // Parent disconnect cleanup must continue through every owned terminal.
    }
  };

  const stop = (acknowledge: boolean) => {
    if (stopped) return;
    stopped = true;
    parentPort.off("message", handle);
    [...sessions.keys()].forEach((terminalId) => finish(terminalId, null, "broker-stopped"));
    if (acknowledge) {
      try {
        post({ kind: "stopped" });
      } catch {
        // Main will observe utility exit or use its bounded kill fallback.
      }
      onShutdownAcknowledged?.();
    }
  };

  const handle = (rawEvent: unknown) => {
    const raw =
      typeof rawEvent === "object" && rawEvent !== null && "data" in rawEvent
        ? rawEvent.data
        : rawEvent;
    const parsed = ptyCommandSchema.safeParse(raw);
    if (!parsed.success) {
      post({
        kind: "error",
        code: "PTY_PROTOCOL_ERROR",
        message: "The PTY broker rejected an invalid command.",
      });
      return;
    }
    const command = parsed.data;
    switch (command.kind) {
      case "create": {
        if (sessions.size >= MAX_SESSIONS) {
          post({
            kind: "error",
            commandId: command.commandId,
            terminalId: command.terminalId,
            code: "PTY_SESSION_LIMIT",
            message: "The PTY session limit has been reached.",
          });
          return;
        }
        const profile = resolveShellProfiles().find(
          (candidate) => candidate.id === command.profileId,
        );
        if (!profile?.available || !profile.executable) {
          post({
            kind: "error",
            commandId: command.commandId,
            terminalId: command.terminalId,
            code: "PTY_PROFILE_UNAVAILABLE",
            message: "The selected terminal profile is unavailable.",
          });
          return;
        }
        try {
          const terminal = spawnPty(profile.executable, [...profile.args], {
            name: "xterm-256color",
            cols: command.cols,
            rows: command.rows,
            cwd: command.cwd,
            env: terminalEnvironment(),
          });
          const session: Session = { terminal, output: new OutputWindow(), sequence: 0 };
          sessions.set(command.terminalId, session);
          post({ kind: "created", commandId: command.commandId, terminalId: command.terminalId });
          terminal.onData((data) => {
            for (const chunk of splitUtf8(data)) {
              if (!sessions.has(command.terminalId)) return;
              if (session.sequence === Number.MAX_SAFE_INTEGER) {
                finish(command.terminalId, null, "output-backpressure");
                return;
              }
              const sequence = ++session.sequence;
              const bytes = utf8Bytes(chunk);
              const decision = session.output.record(sequence, bytes);
              if (decision === "terminate") {
                finish(command.terminalId, null, "output-backpressure");
                return;
              }
              post({
                kind: "output",
                terminalId: command.terminalId,
                sequence: sequenceSchema.parse(String(sequence)),
                data: chunk,
                bytes,
              });
              if (decision === "pause") terminal.pause();
            }
          });
          terminal.onExit(({ exitCode }) => finish(command.terminalId, exitCode, "exited"));
        } catch {
          post({
            kind: "error",
            commandId: command.commandId,
            terminalId: command.terminalId,
            code: "PTY_START_FAILED",
            message: "The terminal process could not be started.",
          });
        }
        return;
      }
      case "write": {
        const session = sessions.get(command.terminalId);
        if (session) session.terminal.write(command.data);
        return;
      }
      case "resize": {
        const session = sessions.get(command.terminalId);
        if (session) session.terminal.resize(command.cols, command.rows);
        return;
      }
      case "ack": {
        const session = sessions.get(command.terminalId);
        if (!session) return;
        const sequence = Number(command.sequence);
        if (Number.isSafeInteger(sequence) && session.output.acknowledge(sequence) === "resume") {
          session.terminal.resume();
        }
        return;
      }
      case "close":
        finish(command.terminalId, null, "closed");
        return;
      case "shutdown":
        stop(true);
        return;
    }
  };

  parentPort.on("message", handle);
  return () => stop(false);
}
