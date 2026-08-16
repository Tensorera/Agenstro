import type { BrowserWindow, WebPreferences } from "electron";

export function secureWebPreferences(preloadPath: string, packaged: boolean): WebPreferences {
  return {
    preload: preloadPath,
    contextIsolation: true,
    sandbox: true,
    nodeIntegration: false,
    nodeIntegrationInWorker: false,
    nodeIntegrationInSubFrames: false,
    webSecurity: true,
    allowRunningInsecureContent: false,
    webviewTag: false,
    devTools: !packaged,
    spellcheck: false,
  };
}

export function installWindowSecurity(window: BrowserWindow, trustedUrl: string): void {
  window.webContents.on("will-navigate", (event, targetUrl) => {
    if (!sameApplicationDocument(targetUrl, trustedUrl)) event.preventDefault();
  });
  window.webContents.on("will-attach-webview", (event) => event.preventDefault());
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.session.setPermissionCheckHandler(() => false);
  window.webContents.session.setPermissionRequestHandler((_webContents, _permission, callback) => {
    callback(false);
  });
}

function sameApplicationDocument(targetUrl: string, trustedUrl: string): boolean {
  try {
    const target = new URL(targetUrl);
    const trusted = new URL(trustedUrl);
    target.hash = "";
    trusted.hash = "";
    return target.href === trusted.href;
  } catch {
    return false;
  }
}
