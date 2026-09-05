import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { TextDecoder } from "node:util";

export interface ProviderInvocation {
  root: string;
  provider: string;
  prompt: string;
  signal: AbortSignal;
}

export interface ProviderResult {
  text: string;
}

export class ProviderCallError extends Error {
  constructor(
    message: string,
    readonly outcome: "failed" | "outcome_unknown",
  ) {
    super(message);
    this.name = "ProviderCallError";
  }
}

const DEFAULT_TIMEOUT_MS = 14_460_000;
const KILL_GRACE_MS = 1_500;
const MAX_FRAME_BYTES = 32 * 1_024 * 1_024;
const MAX_STDOUT_BYTES = 512 * 1_024 * 1_024;
const MAX_REQUEST_BYTES = 16 * 1_024 * 1_024;
const MAX_DIAGNOSTIC_BYTES = 4_096;

type Terminal = { ok: true; text: string } | { ok: false; code: string; message: string };

/** One Tactus-owned invocation. No provider executable or workspace policy is resolved here. */
export function invokeProvider(
  input: ProviderInvocation,
  options: {
    executable?: string;
    commandPrefix?: readonly string[];
    timeoutMs?: number;
  } = {},
): Promise<ProviderResult> {
  if (input.signal.aborted) {
    return Promise.reject(
      new ProviderCallError("Provider invocation was cancelled before dispatch.", "failed"),
    );
  }
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2_147_483_647) {
    return Promise.reject(new ProviderCallError("Invalid provider invocation timeout.", "failed"));
  }
  const id = randomUUID();
  const request = Buffer.from(
    `${JSON.stringify({ api: "agenstro.plugin/v1", id, method: "invoke", params: { prompt: input.prompt } })}\n`,
  );
  if (request.length > MAX_REQUEST_BYTES) {
    return Promise.reject(
      new ProviderCallError("Provider request exceeds the transport limit.", "failed"),
    );
  }

  return new Promise((resolve, reject) => {
    let child;
    try {
      child = spawn(
        options.executable ?? process.env.MOTIVO_TACTUS_BIN ?? process.env.TACTUS_BIN ?? "tactus",
        [
          ...(options.commandPrefix ?? []),
          "dispatch",
          "--namespace",
          "provider",
          "--name",
          input.provider,
          "--root",
          input.root,
        ],
        { cwd: input.root, shell: false, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] },
      );
    } catch {
      reject(
        new ProviderCallError("Could not start Tactus for the provider invocation.", "failed"),
      );
      return;
    }

    let spawned = false;
    let settled = false;
    let closed = false;
    let terminal: Terminal | undefined;
    let interruption: string | undefined;
    let spawnFailed = false;
    let pending: Buffer[] = [];
    let pendingBytes = 0;
    let stdoutBytes = 0;
    let diagnostic = Buffer.alloc(0);
    let killTimer: ReturnType<typeof setTimeout> | undefined;
    const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

    const stop = (message: string): void => {
      if (settled || interruption !== undefined) return;
      interruption = message;
      pending = [];
      pendingBytes = 0;
      if (closed) return;
      child.kill("SIGTERM");
      // Wait for close even after escalation: killing the dispatcher is not
      // evidence that an external provider operation can safely be retried.
      killTimer = setTimeout(() => {
        if (!closed) child.kill("SIGKILL");
      }, KILL_GRACE_MS);
    };
    const abort = (): void =>
      stop("Provider invocation was cancelled after dispatch; its external outcome is unknown.");
    const deadline = setTimeout(
      () =>
        stop(
          "Provider invocation exceeded its transport deadline; its external outcome is unknown.",
        ),
      timeoutMs,
    );

    const finish = (error?: ProviderCallError, value?: ProviderResult): void => {
      if (settled) return;
      settled = true;
      clearTimeout(deadline);
      if (killTimer !== undefined) clearTimeout(killTimer);
      input.signal.removeEventListener("abort", abort);
      if (error) reject(error);
      else if (value) resolve(value);
    };

    const unknown = (message: string): ProviderCallError => {
      const detail = diagnostic.toString("utf8").trim();
      return new ProviderCallError(
        detail ? `${message} Tactus diagnostic: ${detail}` : message,
        "outcome_unknown",
      );
    };

    const frame = (bytes: Buffer): void => {
      if (interruption !== undefined) return;
      try {
        if (terminal !== undefined) throw new Error("Received data after the terminal result.");
        const value: unknown = JSON.parse(decoder.decode(bytes));
        if (!isObject(value) || value.id !== id)
          throw new Error("Provider frame correlation mismatch.");
        if (value.type === "event") {
          if (!isObject(value.event) || typeof value.event.type !== "string") {
            throw new Error("Invalid provider event frame.");
          }
          return; // Progress never accumulates in memory or becomes a task result.
        }
        if (value.type !== "result" || typeof value.ok !== "boolean") {
          throw new Error("Invalid provider terminal frame.");
        }
        if (value.ok) {
          if ("error" in value || !isObject(value.value) || typeof value.value.text !== "string") {
            throw new Error("Provider success must contain a text result and no error.");
          }
          terminal = { ok: true, text: value.value.text };
        } else {
          if (
            "value" in value ||
            !isObject(value.error) ||
            typeof value.error.code !== "string" ||
            typeof value.error.message !== "string"
          ) {
            throw new Error("Invalid provider failure result.");
          }
          terminal = {
            ok: false,
            code: value.error.code,
            message: value.error.message.slice(0, MAX_DIAGNOSTIC_BYTES),
          };
        }
      } catch (error) {
        // Never include raw model output in a protocol-error diagnostic.
        stop(
          error instanceof SyntaxError || error instanceof TypeError
            ? "Provider output was not valid UTF-8 JSONL."
            : error instanceof Error
              ? error.message
              : "Invalid provider output.",
        );
      }
    };

    const append = (bytes: Buffer): boolean => {
      if (pendingBytes + bytes.length > MAX_FRAME_BYTES) {
        stop("Provider output exceeded the JSONL frame limit.");
        return false;
      }
      if (bytes.length) pending.push(bytes);
      pendingBytes += bytes.length;
      return true;
    };

    child.on("spawn", () => {
      spawned = true;
    });
    child.on("error", () => {
      if (!spawned) {
        spawnFailed = true;
        finish(
          new ProviderCallError("Could not start Tactus for the provider invocation.", "failed"),
        );
      } else {
        stop("Tactus process control failed; the provider outcome is unknown.");
      }
    });
    child.stdin.on("error", () =>
      stop("Could not deliver the complete provider request; its outcome is unknown."),
    );
    child.stdout.on("error", () =>
      stop("Could not read the provider result; its outcome is unknown."),
    );
    child.stderr.on("error", () => {
      /* Diagnostics do not determine the invocation result. */
    });
    child.stderr.on("data", (chunk: Buffer) => {
      const remaining = MAX_DIAGNOSTIC_BYTES - diagnostic.length;
      if (remaining > 0) diagnostic = Buffer.concat([diagnostic, chunk.subarray(0, remaining)]);
    });
    child.stdout.on("data", (chunk: Buffer) => {
      if (interruption !== undefined || settled) return;
      stdoutBytes += chunk.length;
      if (stdoutBytes > MAX_STDOUT_BYTES) {
        stop("Provider output exceeded the stream limit.");
        return;
      }
      let start = 0;
      for (;;) {
        const end = chunk.indexOf(10, start);
        if (end < 0) {
          append(chunk.subarray(start));
          return;
        }
        if (!append(chunk.subarray(start, end))) return;
        const complete = Buffer.concat(pending, pendingBytes);
        pending = [];
        pendingBytes = 0;
        frame(complete);
        if (interruption !== undefined) return;
        start = end + 1;
      }
    });
    child.on("close", (code, signal) => {
      closed = true;
      if (spawnFailed || settled) return;
      if (interruption === undefined && pendingBytes > 0) {
        frame(Buffer.concat(pending, pendingBytes));
      }
      if (interruption !== undefined) finish(unknown(interruption));
      else if (terminal === undefined)
        finish(unknown("Tactus closed without an authoritative provider result."));
      else if (!terminal.ok) {
        finish(
          new ProviderCallError(
            terminal.message || "Provider invocation failed.",
            terminal.code === "outcome_unknown" || signal !== null ? "outcome_unknown" : "failed",
          ),
        );
      } else if (code !== 0 || signal !== null) {
        finish(unknown("Tactus did not exit successfully after the provider result."));
      } else finish(undefined, { text: terminal.text });
    });

    input.signal.addEventListener("abort", abort, { once: true });
    if (input.signal.aborted) abort();
    child.stdin.end(request);
  });
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
