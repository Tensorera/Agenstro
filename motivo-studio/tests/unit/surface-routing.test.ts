import type { App, BrowserWindow, Event, IpcMain, IpcMainInvokeEvent } from "electron";
import { describe, expect, it, vi } from "vitest";
import {
  initialSurfaceFromArgv,
  registerSurfaceRouting,
  requestedSurfaceFromArgv,
} from "../../src/main/windows/surface-routing";
import { IPC } from "../../src/shared/ipc";
import { studioSurfaceSchema } from "../../src/shared/surface";

type SecondInstanceListener = (
  event: Event,
  argv: string[],
  workingDirectory: string,
  additionalData: unknown,
) => void;
type IpcHandler = (event: IpcMainInvokeEvent, raw: unknown) => unknown;

describe("Motivo surface routing", () => {
  it("finds scheduler among passthrough argv and defaults malformed requests to files", () => {
    expect(
      requestedSurfaceFromArgv([
        "motivo-studio.exe",
        "--project",
        "project-ignored-by-this-alpha",
        "--surface",
        "scheduler",
        "--another-option",
      ]),
    ).toBe("scheduler");
    expect(requestedSurfaceFromArgv(["motivo-studio", "--surface=files"])).toBe("files");
    expect(initialSurfaceFromArgv(["motivo-studio"])).toBe("files");
    expect(initialSurfaceFromArgv(["motivo-studio", "--surface"])).toBe("files");
    expect(initialSurfaceFromArgv(["motivo-studio", "--surface", "recovery"])).toBe("files");
    expect(
      initialSurfaceFromArgv(["motivo-studio", "https://attacker.invalid/?surface=scheduler"]),
    ).toBe("files");
    expect(
      requestedSurfaceFromArgv([
        "motivo-studio",
        "--surface",
        "scheduler",
        "--surface=https://attacker.invalid/",
      ]),
    ).toBeUndefined();
  });

  it("keeps the public surface schema closed", () => {
    expect(studioSurfaceSchema.safeParse("files").success).toBe(true);
    expect(studioSurfaceSchema.safeParse("scheduler").success).toBe(true);
    expect(studioSurfaceSchema.safeParse("recovery").success).toBe(false);
    expect(studioSurfaceSchema.safeParse("https://attacker.invalid/").success).toBe(false);
    expect(studioSurfaceSchema.safeParse({ url: "motivo://app/index.html" }).success).toBe(false);
  });

  it("routes a valid second instance over typed IPC and restores the existing window", () => {
    const fixture = routingFixture(["motivo-studio"]);

    fixture.emitSecondInstance(["motivo-studio", "--forwarded", "value", "--surface", "scheduler"]);

    expect(fixture.routing.current()).toBe("scheduler");
    expect(fixture.webContents.send).toHaveBeenCalledWith(IPC.surfaceChanged, "scheduler");
    expect(fixture.webContents.executeJavaScript).not.toHaveBeenCalled();
    expect(fixture.window.restore).toHaveBeenCalledOnce();
    expect(fixture.window.show).toHaveBeenCalledOnce();
    expect(fixture.window.focus).toHaveBeenCalledOnce();
  });

  it("rejects unknown second-instance values without changing or focusing the window", () => {
    const fixture = routingFixture(["motivo-studio", "--surface", "files"]);

    fixture.emitSecondInstance(["motivo-studio", "--surface", "https://attacker.invalid/"]);
    fixture.emitSecondInstance(["motivo-studio", "--surface"]);

    expect(fixture.routing.current()).toBe("files");
    expect(fixture.webContents.send).not.toHaveBeenCalled();
    expect(fixture.window.restore).not.toHaveBeenCalled();
    expect(fixture.window.show).not.toHaveBeenCalled();
    expect(fixture.window.focus).not.toHaveBeenCalled();
  });

  it("serves the current surface only to the main frame and disposes every owner listener", () => {
    const fixture = routingFixture(["motivo-studio", "--surface", "scheduler"]);
    const handler = fixture.handlers.get(IPC.surfaceCurrent);
    if (!handler) throw new Error("expected the current-surface IPC handler");

    expect(handler(fixture.trustedEvent, {})).toEqual({ ok: true, data: "scheduler" });
    expect(handler(fixture.trustedEvent, { url: "https://attacker.invalid/" })).toMatchObject({
      ok: false,
      error: { code: "IPC_INVALID_ARGUMENT" },
    });
    expect(
      handler({ ...fixture.trustedEvent, senderFrame: {} } as IpcMainInvokeEvent, {}),
    ).toMatchObject({
      ok: false,
      error: { code: "IPC_SOURCE_REJECTED" },
    });

    fixture.routing.dispose();
    fixture.routing.dispose();
    expect(fixture.app.removeListener).toHaveBeenCalledOnce();
    expect(fixture.ipcMain.removeHandler).toHaveBeenCalledOnce();
    expect(fixture.ipcMain.removeHandler).toHaveBeenCalledWith(IPC.surfaceCurrent);
  });
});

function routingFixture(initialArgv: readonly string[]) {
  let secondInstanceListener: SecondInstanceListener | undefined;
  const handlers = new Map<string, IpcHandler>();
  const app = {
    on: vi.fn((event: string, listener: SecondInstanceListener) => {
      if (event === "second-instance") secondInstanceListener = listener;
    }),
    removeListener: vi.fn(),
  } as unknown as App;
  const ipcMain = {
    handle: vi.fn((channel: string, handler: IpcHandler) => handlers.set(channel, handler)),
    removeHandler: vi.fn((channel: string) => handlers.delete(channel)),
  } as unknown as IpcMain;
  const mainFrame = {};
  const webContents = {
    id: 17,
    mainFrame,
    isDestroyed: vi.fn(() => false),
    send: vi.fn(),
    executeJavaScript: vi.fn(),
  };
  const window = {
    webContents,
    isDestroyed: vi.fn(() => false),
    isMinimized: vi.fn(() => true),
    isVisible: vi.fn(() => false),
    restore: vi.fn(),
    show: vi.fn(),
    focus: vi.fn(),
  } as unknown as BrowserWindow;
  const routing = registerSurfaceRouting({
    app,
    ipcMain,
    initialArgv,
    getWindow: () => window,
  });
  const trustedEvent = {
    sender: webContents,
    senderFrame: mainFrame,
  } as unknown as IpcMainInvokeEvent;

  return {
    app,
    emitSecondInstance: (argv: string[]) => {
      if (!secondInstanceListener) throw new Error("expected the second-instance listener");
      secondInstanceListener({} as Event, argv, "D:\\project", {});
    },
    handlers,
    ipcMain,
    routing,
    trustedEvent,
    webContents,
    window,
  };
}
