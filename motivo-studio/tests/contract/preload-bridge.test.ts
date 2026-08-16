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

describe("preload bridge", () => {
  beforeEach(() => {
    electron.exposed = undefined;
    electron.listeners.clear();
    electron.contextBridge.exposeInMainWorld.mockClear();
    electron.ipcRenderer.invoke.mockReset();
    electron.ipcRenderer.on.mockClear();
    electron.ipcRenderer.removeListener.mockClear();
  });

  it("exposes a narrow API and passes no root to workspace operations", async () => {
    const bridge = installedBridge();
    electron.ipcRenderer.invoke.mockResolvedValueOnce({ ok: true, data: null });
    await expect(bridge.studio.openInitialized()).resolves.toBeNull();
    expect(electron.ipcRenderer.invoke).toHaveBeenCalledWith(IPC.studioOpenInitialized, {});
    expect(Object.keys(bridge).sort()).toEqual(["actions", "runs", "studio"]);
  });

  it("forwards only validated action events and releases the single listener", () => {
    const bridge = installedBridge();
    const listener = vi.fn();
    const unsubscribe = bridge.actions.subscribe(listener);
    const ipcListener = electron.listeners.get(IPC.actionEvent);
    if (!ipcListener) throw new Error("expected action event listener");
    ipcListener(
      {},
      {
        type: "started",
        actionId: "d7ef7a0c-63c6-4f33-8312-8a0c463f675d",
        kind: "check",
        startedAtUnixMs: "1",
      },
    );
    ipcListener({}, { type: "output", root: "D:\\private" });
    expect(listener).toHaveBeenCalledOnce();
    unsubscribe();
    unsubscribe();
    expect(electron.ipcRenderer.removeListener).toHaveBeenCalledOnce();
  });

  it("rejects renderer-supplied extra action fields before IPC", async () => {
    const bridge = installedBridge();
    await expect(
      bridge.actions.start({ kind: "run", root: "D:\\private" } as never),
    ).rejects.toThrow();
    expect(electron.ipcRenderer.invoke).not.toHaveBeenCalled();
  });
});

function installedBridge(): MotivoBridge {
  installBridge();
  if (!electron.exposed) throw new Error("expected Motivo bridge");
  return electron.exposed as MotivoBridge;
}
