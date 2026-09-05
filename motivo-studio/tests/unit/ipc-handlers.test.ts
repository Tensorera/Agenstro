import type { BrowserWindow, Dialog, IpcMain, IpcMainInvokeEvent } from "electron";
import { describe, expect, it, vi } from "vitest";
import { registerIpcHandlers, type StudioController } from "../../src/main/ipc/handlers";
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

  it("validates and routes session answers through the same main-frame boundary", async () => {
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
    const webContents = { mainFrame, isDestroyed: () => false, send: vi.fn() };
    const window = { webContents, isDestroyed: () => false } as unknown as BrowserWindow;
    const dialog = { showOpenDialog: vi.fn() } as unknown as Pick<Dialog, "showOpenDialog">;
    const answered = {
      api: "agenstro.session/v1" as const,
      sessionId: "session-desk-1",
      label: "Desk build",
      state: "planning" as const,
      turn: "3",
      answered: [
        {
          axis: "desk.frame",
          option: "fixed",
          label: "Fixed height",
          defaulted: false,
          answeredAtUnixMs: "2",
        },
      ],
      startedUnixMs: "1",
      updatedUnixMs: "2",
    };
    const controller = {
      current: vi.fn().mockReturnValue(null),
      open: vi.fn(),
      initialize: vi.fn(),
      refresh: vi.fn(),
      events: vi.fn(),
      sessions: vi.fn(),
      session: vi.fn(),
      answer: vi.fn().mockResolvedValue(answered),
      taskList: vi.fn().mockResolvedValue([]),
      taskCurrent: vi.fn(),
      taskCreate: vi.fn(),
      taskContinue: vi.fn(),
      taskPause: vi.fn(),
      start: vi.fn(),
      cancel: vi.fn(),
      dispose: vi.fn(),
    } as unknown as StudioController;
    const unregister = registerIpcHandlers({ ipcMain, window, dialog, controller });
    const answer = handlers.get(IPC.sessionAnswer);
    if (!answer) throw new Error("session answer handler was not registered");
    const sender = { sender: webContents, senderFrame: mainFrame } as never;
    const input = {
      workspaceHandle: "aa665bbe-ece0-40e6-8235-2278635aee84",
      sessionId: "session-desk-1",
      turn: "3",
      axis: "desk.frame",
      option: "fixed",
      note: "Keep it repairable.",
    };

    await expect(answer(sender, input)).resolves.toEqual({ ok: true, data: answered });
    expect(controller.answer).toHaveBeenCalledWith(input);
    await expect(answer(sender, { ...input, root: "D:\\private" })).resolves.toMatchObject({
      ok: false,
      error: { category: "internal" },
    });
    expect(controller.answer).toHaveBeenCalledOnce();

    const tasks = handlers.get(IPC.taskList);
    if (!tasks) throw new Error("task list handler was not registered");
    await expect(tasks(sender, { workspaceHandle: input.workspaceHandle })).resolves.toEqual({
      ok: true,
      data: [],
    });
    expect(controller.taskList).toHaveBeenCalledWith({ workspaceHandle: input.workspaceHandle });
    await expect(
      tasks(sender, { workspaceHandle: input.workspaceHandle, root: "/other" }),
    ).resolves.toMatchObject({ ok: false });
    await expect(
      tasks({ sender: webContents, senderFrame: { routingId: 2 } } as never, {
        workspaceHandle: input.workspaceHandle,
      }),
    ).resolves.toMatchObject({ ok: false });
    expect(controller.taskList).toHaveBeenCalledOnce();
    unregister();
  });
});
