import { beforeEach, describe, expect, it, vi } from "vitest";
import { installBridge } from "../../src/preload/bridge";
import type { MotivoBridge } from "../../src/shared/contracts";
import { IPC } from "../../src/shared/ipc";

const electron = vi.hoisted(() => {
  const listeners = new Map<string, (...values: unknown[]) => void>();
  return {
    exposed: undefined as unknown,
    listeners,
    contextBridge: {
      exposeInMainWorld: vi.fn((_name: string, value: unknown) => {
        electron.exposed = value;
      }),
    },
    ipcRenderer: {
      invoke: vi.fn(),
      on: vi.fn((channel: string, listener: (...values: unknown[]) => void) => {
        listeners.set(channel, listener);
      }),
      removeListener: vi.fn((channel: string, listener: (...values: unknown[]) => void) => {
        if (listeners.get(channel) === listener) listeners.delete(channel);
      }),
    },
  };
});

vi.mock("electron", () => ({
  contextBridge: electron.contextBridge,
  ipcRenderer: electron.ipcRenderer,
}));

describe("surface preload boundary", () => {
  beforeEach(() => {
    electron.exposed = undefined;
    electron.listeners.clear();
    electron.contextBridge.exposeInMainWorld.mockClear();
    electron.ipcRenderer.invoke.mockReset();
    electron.ipcRenderer.on.mockClear();
    electron.ipcRenderer.removeListener.mockClear();
  });

  it("reads the initial enum through bounded IPC and rejects an arbitrary URL response", async () => {
    const bridge = installedBridge();
    electron.ipcRenderer.invoke.mockResolvedValueOnce({ ok: true, data: "scheduler" });

    await expect(bridge.surface.current()).resolves.toBe("scheduler");
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(IPC.surfaceCurrent, {});

    electron.ipcRenderer.invoke.mockResolvedValueOnce({
      ok: true,
      data: "https://attacker.invalid/",
    });
    await expect(bridge.surface.current()).rejects.toThrow();
  });

  it("forwards only validated surface events and removes its exact listener idempotently", () => {
    const bridge = installedBridge();
    const listener = vi.fn();
    const unsubscribe = bridge.surface.subscribe(listener);
    const ipcListener = electron.listeners.get(IPC.surfaceChanged);
    if (!ipcListener) throw new Error("expected the surface IPC listener");

    ipcListener({}, "scheduler");
    ipcListener({}, "recovery");
    ipcListener({}, "https://attacker.invalid/");
    ipcListener({}, { surface: "scheduler" });

    expect(listener).toHaveBeenCalledOnce();
    expect(listener).toHaveBeenCalledWith("scheduler");
    unsubscribe();
    unsubscribe();
    expect(electron.ipcRenderer.removeListener).toHaveBeenCalledOnce();
    expect(electron.listeners.has(IPC.surfaceChanged)).toBe(false);
  });

  it("allows only one app-level surface listener and releases the slot on unsubscribe", () => {
    const bridge = installedBridge();
    const firstUnsubscribe = bridge.surface.subscribe(vi.fn());

    expect(() => bridge.surface.subscribe(vi.fn())).toThrow(RangeError);
    firstUnsubscribe();

    const replacementUnsubscribe = bridge.surface.subscribe(vi.fn());
    replacementUnsubscribe();
    expect(electron.ipcRenderer.removeListener).toHaveBeenCalledTimes(2);
  });
});

function installedBridge(): MotivoBridge {
  installBridge();
  if (electron.exposed === undefined) throw new Error("expected the Motivo preload bridge");
  return electron.exposed as MotivoBridge;
}
