import { PassThrough, Writable } from "node:stream";
import { describe, expect, it, vi } from "vitest";
import {
  DaemonBootstrapper,
  type DaemonLauncher,
  type LaunchedProcess,
} from "../../src/main/daemon/bootstrap";

class FakeLauncher implements DaemonLauncher {
  readonly executables: string[] = [];
  readonly requests: Array<Record<string, unknown>> = [];

  launch(executable: string): LaunchedProcess {
    this.executables.push(executable);
    const input = new PassThrough();
    const output = new PassThrough();
    let resolveExit: (() => void) | undefined;
    const exited = new Promise<void>((resolve) => {
      resolveExit = resolve;
    });
    input.once("data", (frame: Buffer) => {
      const length = frame.readUInt32BE(0);
      this.requests.push(JSON.parse(frame.subarray(4, 4 + length).toString("utf8")));
      const payload = Buffer.from(
        JSON.stringify({
          endpoint: `127.0.0.1:${String(41000 + this.requests.length)}`,
          instanceId: `daemon-${String(this.requests.length)}`,
          apiMajor: 1,
          apiMinor: 0,
          startupNonce: "0123456789abcdef",
        }),
      );
      const response = Buffer.alloc(4 + payload.byteLength);
      response.writeUInt32BE(payload.byteLength, 0);
      payload.copy(response, 4);
      output.write(response);
    });
    input.once("finish", () => resolveExit?.());
    return { input, output, exited, terminate: vi.fn(() => resolveExit?.()) };
  }
}

class FailingInput extends Writable {
  frame: Buffer | undefined;
  tokenAtWrite: string | undefined;

  constructor(private readonly mode: "callback" | "throw") {
    super();
    this.on("error", () => undefined);
  }

  override _write(
    chunk: Buffer,
    _encoding: BufferEncoding,
    callback: (error?: Error | null) => void,
  ): void {
    this.frame = chunk;
    const length = chunk.readUInt32BE(0);
    const request = JSON.parse(chunk.subarray(4, 4 + length).toString("utf8")) as {
      token?: string;
    };
    this.tokenAtWrite = request.token;
    if (this.mode === "throw") {
      throw new Error("synchronous bootstrap write failure");
    }
    queueMicrotask(() => callback(new Error("bootstrap write callback failure")));
  }
}

describe("per-launch daemon bootstrap", () => {
  it("uses a unique 256-bit pipe token without exposing transport arguments", async () => {
    const launcher = new FakeLauncher();
    let tokenByte = 0;
    const bootstrapper = new DaemonBootstrapper(
      launcher,
      () => Buffer.alloc(32, ++tokenByte),
      () => `client-${String(tokenByte)}`,
    );
    const first = await bootstrapper.launch("D:\\bundle\\agentrod.exe", "parent-1", "clef-sdk");
    const second = await bootstrapper.launch(
      "D:\\bundle\\tactusd.exe",
      "parent-1",
      "tactus-runtime",
    );

    expect(launcher.executables).toEqual(["D:\\bundle\\agentrod.exe", "D:\\bundle\\tactusd.exe"]);
    expect(launcher.requests).toHaveLength(2);
    expect(launcher.requests[0]?.token).not.toBe(launcher.requests[1]?.token);
    expect(Buffer.from(String(launcher.requests[0]?.token), "base64url")).toHaveLength(32);
    expect(first.connection.endpoint).toBe("127.0.0.1:41001");
    expect(first.connection.product).toBe("clef-sdk");

    const firstToken = first.connection.token;
    await Promise.all([first.close(), second.close()]);
    expect([...firstToken]).toEqual(new Array<number>(32).fill(0));
  });

  it.each(["callback", "throw"] as const)(
    "zeroes the request frame and token when the bootstrap write fails by %s",
    async (mode) => {
      const token = Buffer.alloc(32, 0xa5);
      const encodedToken = token.toString("base64url");
      const input = new FailingInput(mode);
      const output = new PassThrough();
      const terminate = vi.fn();
      const launcher: DaemonLauncher = {
        launch: () => ({
          input,
          output,
          exited: new Promise<void>(() => undefined),
          terminate,
        }),
      };
      const bootstrapper = new DaemonBootstrapper(
        launcher,
        () => token,
        () => "client-1",
      );

      await expect(
        bootstrapper.launch("D:\\bundle\\agentrod.exe", "parent-1", "clef-sdk"),
      ).rejects.toThrow(`bootstrap write ${mode === "callback" ? "callback" : "failure"}`);

      const frame = input.frame;
      expect(frame).toBeDefined();
      if (frame === undefined) throw new Error("Bootstrap request frame was not written.");
      expect(input.tokenAtWrite).toBe(encodedToken);
      expect([...frame]).toEqual(new Array<number>(frame.byteLength).fill(0));
      expect([...token]).toEqual(new Array<number>(32).fill(0));
      expect(input.destroyed).toBe(true);
      expect(output.destroyed).toBe(true);
      expect(terminate).toHaveBeenCalledOnce();
    },
  );
});
