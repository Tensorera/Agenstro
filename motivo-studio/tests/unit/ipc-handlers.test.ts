import type { BrowserWindow, Dialog, IpcMain, IpcMainInvokeEvent } from "electron";
import { describe, expect, it, vi } from "vitest";
import { registerIpcHandlers } from "../../src/main/ipc/handlers";
import { IPC } from "../../src/shared/ipc";

describe("main IPC sender boundary", () => {
  it("accepts only the window main frame and unregisters every fixed channel", async () => {
    const handlers = new Map<
      string,
      (event: IpcMainInvokeEvent, input: unknown) => Promise<unknown>
    >();
    const ipcMain = {
      handle: vi.fn(
        (
          channel: string,
          handler: (event: IpcMainInvokeEvent, input: unknown) => Promise<unknown>,
        ) => handlers.set(channel, handler),
      ),
      removeHandler: vi.fn((channel: string) => handlers.delete(channel)),
    } as unknown as IpcMain;
    const mainFrame = { routingId: 1 };
    const webContents = {
      mainFrame,
      isDestroyed: () => false,
      send: vi.fn(),
    };
    const window = {
      webContents,
      isDestroyed: () => false,
    } as unknown as BrowserWindow;
    const dialog = { showOpenDialog: vi.fn() } as unknown as Pick<Dialog, "showOpenDialog">;
    const unregister = registerIpcHandlers({ ipcMain, window, dialog });
    const current = handlers.get(IPC.studioCurrent);
    if (!current) throw new Error("current handler was not registered");

    await expect(
      current({ sender: webContents, senderFrame: mainFrame } as never, {}),
    ).resolves.toEqual({ ok: true, data: null });
    await expect(
      current({ sender: webContents, senderFrame: { routingId: 2 } } as never, {}),
    ).resolves.toMatchObject({ ok: false, error: { code: "internal_error" } });

    unregister();
    expect(ipcMain.removeHandler).toHaveBeenCalledTimes(Object.keys(IPC).length - 1);
    expect(handlers.size).toBe(0);
  });
});
