import type { BrowserWindow } from "electron";
import { describe, expect, it, vi } from "vitest";
import { installWindowSecurity, secureWebPreferences } from "../../src/main/windows/security";

describe("Electron window security", () => {
  it("pins renderer sandbox preferences explicitly", () => {
    expect(secureWebPreferences("D:\\app\\preload.js", true)).toMatchObject({
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      nodeIntegrationInWorker: false,
      nodeIntegrationInSubFrames: false,
      webSecurity: true,
      allowRunningInsecureContent: false,
      webviewTag: false,
      devTools: false,
    });
  });

  it("denies navigation, new windows, webviews, and every permission", () => {
    const handlers = new Map<string, (...values: unknown[]) => void>();
    let openHandler: (() => { action: string }) | undefined;
    let permissionCheck: (() => boolean) | undefined;
    let permissionRequest:
      | ((webContents: unknown, permission: string, callback: (allowed: boolean) => void) => void)
      | undefined;
    const fake = {
      webContents: {
        on: vi.fn((name: string, handler: (...values: unknown[]) => void) =>
          handlers.set(name, handler),
        ),
        setWindowOpenHandler: vi.fn((handler: () => { action: string }) => {
          openHandler = handler;
        }),
        session: {
          setPermissionCheckHandler: vi.fn((handler: () => boolean) => {
            permissionCheck = handler;
          }),
          setPermissionRequestHandler: vi.fn(
            (
              handler: (
                webContents: unknown,
                permission: string,
                callback: (allowed: boolean) => void,
              ) => void,
            ) => {
              permissionRequest = handler;
            },
          ),
        },
      },
    };
    installWindowSecurity(fake as unknown as BrowserWindow, "file:///app/index.html");

    const navigation = { preventDefault: vi.fn() };
    handlers.get("will-navigate")?.(navigation, "https://attacker.invalid/");
    expect(navigation.preventDefault).toHaveBeenCalledOnce();
    const sameDocument = { preventDefault: vi.fn() };
    handlers.get("will-navigate")?.(sameDocument, "file:///app/index.html#run");
    expect(sameDocument.preventDefault).not.toHaveBeenCalled();
    const webview = { preventDefault: vi.fn() };
    handlers.get("will-attach-webview")?.(webview);
    expect(webview.preventDefault).toHaveBeenCalledOnce();
    expect(openHandler?.()).toEqual({ action: "deny" });
    expect(permissionCheck?.()).toBe(false);
    const permissionCallback = vi.fn();
    permissionRequest?.(null, "clipboard-read", permissionCallback);
    expect(permissionCallback).toHaveBeenCalledWith(false);
  });
});
