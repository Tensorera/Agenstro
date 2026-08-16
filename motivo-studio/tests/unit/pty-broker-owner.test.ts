import { EventEmitter } from "node:events";
import type { IPty } from "node-pty";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const electronMock = vi.hoisted(() => ({ fork: vi.fn() }));

vi.mock("electron", () => ({
  utilityProcess: { fork: electronMock.fork },
}));

import { UtilityProcessPtyBroker } from "../../src/main/pty/pty-broker";
import { runPtyHost } from "../../src/pty/host";
import type { PtySpawn } from "../../src/pty/node-pty-loader";
import { resolveShellProfiles } from "../../src/pty/shell-profiles";

class FakeUtilityProcess extends EventEmitter {
  readonly postMessage = vi.fn();
  readonly kill = vi.fn();
}

class FakeParentPort {
  readonly postMessage = vi.fn();
  private readonly listeners = new Set<(event: unknown) => void>();

  on(_event: "message", listener: (event: unknown) => void): void {
    this.listeners.add(listener);
  }

  off(_event: "message", listener: (event: unknown) => void): void {
    this.listeners.delete(listener);
  }

  emitMessage(message: unknown): void {
    this.listeners.forEach((listener) => listener(message));
  }

  get listenerCount(): number {
    return this.listeners.size;
  }
}

describe("PTY broker ownership", () => {
  let child: FakeUtilityProcess;

  beforeEach(() => {
    child = new FakeUtilityProcess();
    electronMock.fork.mockReturnValue(child);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("closes a late PTY when its create request times out", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");
    const created = broker.create(input());
    const rejected = expect(created).rejects.toThrow(/deadline/);

    await vi.advanceTimersByTimeAsync(5_000);
    await rejected;

    const createCommand = child.postMessage.mock.calls[0]?.[0];
    expect(createCommand).toMatchObject({ kind: "create" });
    expect(child.postMessage.mock.calls[1]?.[0]).toEqual({
      kind: "close",
      terminalId: createCommand.terminalId,
    });
    child.emit("message", {
      kind: "created",
      commandId: createCommand.commandId,
      terminalId: createCommand.terminalId,
    });
    expect(child.postMessage.mock.calls[2]?.[0]).toEqual({
      kind: "close",
      terminalId: createCommand.terminalId,
    });
    const shutdown = broker.shutdown();
    child.emit("message", { kind: "stopped" });
    await shutdown;
  });

  it("bounds concurrent create ownership before utility replies", async () => {
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");
    const pending = Array.from({ length: 8 }, () => broker.create(input()));
    const settled = Promise.allSettled(pending);

    await expect(broker.create(input())).rejects.toThrow(/limit/);
    const shutdown = broker.shutdown();
    child.emit("message", { kind: "stopped" });
    await shutdown;
    await settled;
    expect(child.postMessage).toHaveBeenCalledTimes(9);
  });

  it("awaits a typed stopped acknowledgement without killing the utility", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");

    const shutdown = broker.shutdown();

    expect(child.postMessage).toHaveBeenCalledExactlyOnceWith({ kind: "shutdown" });
    expect(child.kill).not.toHaveBeenCalled();
    child.emit("message", { kind: "stopped" });
    await shutdown;
    expect(child.kill).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    expect(child.listenerCount("message")).toBe(0);
    expect(child.listenerCount("exit")).toBe(0);
  });

  it("accepts utility exit before acknowledgement as graceful completion", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");

    const shutdown = broker.shutdown();
    child.emit("exit", 0);

    await shutdown;
    expect(child.kill).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    expect(child.listenerCount("message")).toBe(0);
    expect(child.listenerCount("exit")).toBe(0);
  });

  it("kills the utility only after the graceful shutdown deadline", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");

    const shutdown = broker.shutdown();
    await vi.advanceTimersByTimeAsync(1_999);
    expect(child.kill).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    await shutdown;
    expect(child.kill).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    expect(child.listenerCount("message")).toBe(0);
    expect(child.listenerCount("exit")).toBe(0);
  });

  it("kills the utility and clears its deadline when shutdown cannot be sent", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    child.postMessage.mockImplementationOnce(() => {
      throw new Error("utility transport unavailable");
    });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");

    await broker.shutdown();

    expect(child.kill).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    expect(child.listenerCount("message")).toBe(0);
    expect(child.listenerCount("exit")).toBe(0);
  });

  it("shares one shutdown operation across repeated callers", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");

    const first = broker.shutdown();
    const second = broker.shutdown();

    expect(first).toBe(second);
    expect(child.postMessage).toHaveBeenCalledExactlyOnceWith({ kind: "shutdown" });
    expect(child.kill).not.toHaveBeenCalled();
    child.emit("message", { kind: "stopped" });
    await Promise.all([first, second]);
  });

  it("rejects pending and new creates while graceful shutdown is in flight", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const broker = new UtilityProcessPtyBroker("pty-host.js", "node-pty");
    const pending = broker.create(input());
    const rejected = expect(pending).rejects.toThrow(/stopped/);

    const shutdown = broker.shutdown();

    await rejected;
    await expect(broker.create(input())).rejects.toThrow(/unavailable/);
    expect(child.postMessage.mock.calls.map(([command]) => command.kind)).toEqual([
      "create",
      "shutdown",
    ]);
    expect(child.kill).not.toHaveBeenCalled();
    child.emit("message", { kind: "stopped" });
    await shutdown;
    expect(vi.getTimerCount()).toBe(0);
  });

  it("kills every owned terminal before acknowledging utility shutdown", () => {
    const profile = resolveShellProfiles().find((candidate) => candidate.available);
    expect(profile).toBeDefined();
    if (!profile) return;
    const parentPort = new FakeParentPort();
    const firstTerminal = fakeTerminal();
    const secondTerminal = fakeTerminal();
    const terminals = [firstTerminal, secondTerminal];
    const spawnPty = vi
      .fn<PtySpawn>()
      .mockReturnValueOnce(firstTerminal)
      .mockReturnValueOnce(secondTerminal);
    const onShutdownAcknowledged = vi.fn();
    const stop = runPtyHost(parentPort, spawnPty, onShutdownAcknowledged);
    const commandIds = [
      "00000000-0000-4000-8000-000000000001",
      "00000000-0000-4000-8000-000000000002",
    ];
    const terminalIds = [
      "00000000-0000-4000-8000-000000000003",
      "00000000-0000-4000-8000-000000000004",
    ];
    terminalIds.forEach((terminalId, index) => {
      parentPort.emitMessage({
        kind: "create",
        commandId: commandIds[index],
        terminalId,
        profileId: profile.id,
        cwd: process.cwd(),
        cols: 80,
        rows: 24,
      });
    });

    parentPort.emitMessage({ kind: "shutdown" });

    terminals.forEach((terminal) => expect(terminal.kill).toHaveBeenCalledOnce());
    expect(parentPort.postMessage).toHaveBeenLastCalledWith({ kind: "stopped" });
    expect(onShutdownAcknowledged).toHaveBeenCalledOnce();
    expect(parentPort.listenerCount).toBe(0);
    stop();
    terminals.forEach((terminal) => expect(terminal.kill).toHaveBeenCalledOnce());
  });
});

function input() {
  return {
    cwd: "C:\\workspace",
    profileId: "powershell" as const,
    cols: 80,
    rows: 24,
  };
}

function fakeTerminal(): IPty {
  return {
    write: vi.fn(),
    resize: vi.fn(),
    kill: vi.fn(),
    pause: vi.fn(),
    resume: vi.fn(),
    onData: vi.fn(),
    onExit: vi.fn(),
  } as unknown as IPty;
}
