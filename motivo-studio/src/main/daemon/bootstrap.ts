import { randomBytes, randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { Readable, Writable } from "node:stream";
import { spawn, type ChildProcess } from "node:child_process";
import { z } from "zod";
import type { DaemonConnection, DaemonConnections, DaemonProduct } from "./grpc-daemon-client";
import { StudioFault } from "../errors";

const MAX_BOOTSTRAP_FRAME_BYTES = 16_384;
const BOOTSTRAP_TIMEOUT_MS = 8_000;

const bootstrapResponseSchema = z
  .object({
    endpoint: z.string().regex(/^127\.0\.0\.1:([1-9][0-9]{0,4})$/),
    instanceId: z.string().min(1).max(128),
    apiMajor: z.number().int().min(1).max(255),
    apiMinor: z.number().int().min(0).max(65_535),
    startupNonce: z.string().min(16).max(256),
  })
  .strict();

export interface LaunchedProcess {
  readonly input: Writable;
  readonly output: Readable;
  readonly exited: Promise<void>;
  terminate(): void;
}

export interface DaemonLauncher {
  launch(executable: string): LaunchedProcess;
}

interface OwnedDaemon {
  readonly connection: DaemonConnection;
  close(): Promise<void>;
}

export interface BootstrappedDaemons {
  readonly connections: DaemonConnections;
  close(): Promise<void>;
}

export class NodeDaemonLauncher implements DaemonLauncher {
  launch(executable: string): LaunchedProcess {
    if (!isAbsolute(executable) || !existsSync(executable)) {
      throw new StudioFault({
        code: "DAEMON_BINARY_MISSING",
        category: "connection",
        retryable: false,
        message: "A matching bundled daemon binary is unavailable.",
      });
    }
    const child = spawn(executable, [], {
      shell: false,
      windowsHide: true,
      stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"],
      env: minimalDaemonEnvironment(),
    });
    const input = child.stdio[3];
    const output = child.stdio[4];
    if (!(input instanceof Writable) || !(output instanceof Readable)) {
      child.kill();
      throw new Error("Daemon bootstrap pipes were not created.");
    }
    return {
      input,
      output,
      exited: childExit(child),
      terminate: () => child.kill(),
    };
  }
}

export class DaemonBootstrapper {
  constructor(
    private readonly launcher: DaemonLauncher,
    private readonly createToken: () => Buffer = () => randomBytes(32),
    private readonly createInstanceId: () => string = randomUUID,
  ) {}

  async launch(
    executable: string,
    parentInstanceId: string,
    product: DaemonProduct,
  ): Promise<OwnedDaemon> {
    const token = this.createToken();
    if (token.byteLength !== 32) {
      token.fill(0);
      throw new Error("Daemon bootstrap tokens must contain exactly 256 bits.");
    }
    const child = this.launcher.launch(executable);
    try {
      const request = encodeFrame({
        token: token.toString("base64url"),
        parentInstanceId,
        clientInstanceId: this.createInstanceId(),
        protocol: { major: 1, minimumMinor: 0, maximumMinor: 0 },
      });
      try {
        await writeFrame(child.input, request);
      } finally {
        request.fill(0);
      }
      const response = bootstrapResponseSchema.parse(
        JSON.parse(await readFrame(child.output, BOOTSTRAP_TIMEOUT_MS)),
      );
      if (response.apiMajor !== 1) {
        throw new Error("Daemon API major version is incompatible.");
      }
      let closed = false;
      return {
        connection: {
          endpoint: response.endpoint,
          instanceId: response.instanceId,
          token,
          product,
        },
        close: async () => {
          if (closed) return;
          closed = true;
          token.fill(0);
          child.input.end();
          const stopped = await Promise.race([
            child.exited.then(() => true),
            delay(2_000).then(() => false),
          ]);
          if (!stopped) {
            child.terminate();
            await Promise.race([child.exited, delay(1_000)]);
          }
        },
      };
    } catch (error) {
      token.fill(0);
      child.input.destroy();
      child.output.destroy();
      child.terminate();
      throw error;
    }
  }
}

export async function startBundledDaemons(
  resourcesPath: string,
  launcher: DaemonLauncher = new NodeDaemonLauncher(),
): Promise<BootstrappedDaemons> {
  const extension = process.platform === "win32" ? ".exe" : "";
  const daemonDirectory = join(resourcesPath, "daemons");
  const bootstrapper = new DaemonBootstrapper(launcher);
  const parentInstanceId = randomUUID();
  const agentrod = await bootstrapper.launch(
    join(daemonDirectory, `agentrod${extension}`),
    parentInstanceId,
    "clef-sdk",
  );
  let tactusd: OwnedDaemon;
  try {
    tactusd = await bootstrapper.launch(
      join(daemonDirectory, `tactusd${extension}`),
      parentInstanceId,
      "tactus-runtime",
    );
  } catch (error) {
    await agentrod.close();
    throw error;
  }
  return {
    connections: { agentrod: agentrod.connection, tactusd: tactusd.connection },
    close: async () => {
      await Promise.all([agentrod.close(), tactusd.close()]);
    },
  };
}

function encodeFrame(value: unknown): Buffer {
  const payload = Buffer.from(JSON.stringify(value), "utf8");
  if (payload.byteLength > MAX_BOOTSTRAP_FRAME_BYTES) {
    payload.fill(0);
    throw new Error("Daemon bootstrap frame exceeds its size limit.");
  }
  const frame = Buffer.allocUnsafe(4 + payload.byteLength);
  frame.writeUInt32BE(payload.byteLength, 0);
  payload.copy(frame, 4);
  payload.fill(0);
  return frame;
}

function writeFrame(stream: Writable, frame: Buffer): Promise<void> {
  return new Promise((resolve, reject) => {
    stream.write(frame, (error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

function readFrame(stream: Readable, timeoutMs: number): Promise<string> {
  return new Promise((resolve, reject) => {
    let buffered = Buffer.alloc(0);
    let expected: number | undefined;
    const timer = setTimeout(() => finish(new Error("Daemon bootstrap timed out.")), timeoutMs);
    const cleanup = () => {
      clearTimeout(timer);
      stream.off("data", onData);
      stream.off("error", finish);
      stream.off("end", onEnd);
    };
    const finish = (error?: Error, value?: string) => {
      cleanup();
      buffered.fill(0);
      if (error) reject(error);
      else resolve(value ?? "");
    };
    const onEnd = () => finish(new Error("Daemon bootstrap pipe closed early."));
    const onData = (chunk: Buffer) => {
      if (buffered.byteLength + chunk.byteLength > MAX_BOOTSTRAP_FRAME_BYTES + 4) {
        finish(new Error("Daemon bootstrap response exceeds its size limit."));
        return;
      }
      buffered = Buffer.concat([buffered, chunk]);
      if (expected === undefined && buffered.byteLength >= 4) {
        expected = buffered.readUInt32BE(0);
        if (expected === 0 || expected > MAX_BOOTSTRAP_FRAME_BYTES) {
          finish(new Error("Daemon bootstrap frame length is invalid."));
          return;
        }
      }
      if (expected !== undefined && buffered.byteLength >= expected + 4) {
        if (buffered.byteLength !== expected + 4) {
          finish(new Error("Daemon bootstrap returned trailing bytes."));
          return;
        }
        finish(undefined, buffered.subarray(4).toString("utf8"));
      }
    };
    stream.on("data", onData);
    stream.once("error", finish);
    stream.once("end", onEnd);
  });
}

function childExit(child: ChildProcess): Promise<void> {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) {
      resolve();
      return;
    }
    child.once("exit", () => resolve());
  });
}

function minimalDaemonEnvironment(): NodeJS.ProcessEnv {
  const environment: NodeJS.ProcessEnv = {};
  for (const key of ["SystemRoot", "WINDIR", "PATH", "PATHEXT", "TEMP", "TMP", "HOME", "LANG"]) {
    const value = process.env[key];
    if (value !== undefined) environment[key] = value;
  }
  return environment;
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
