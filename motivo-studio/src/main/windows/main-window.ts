import { join } from "node:path";
import { app, BrowserWindow } from "electron";
import { installWindowSecurity, secureWebPreferences } from "./security";
import { installWindowZoom } from "./zoom";

declare const MAIN_WINDOW_VITE_DEV_SERVER_URL: string | undefined;

export function createMainWindow(): BrowserWindow {
  const preloadPath = join(__dirname, "preload.js");
  const window = new BrowserWindow({
    width: 1440,
    height: 920,
    minWidth: 760,
    minHeight: 560,
    show: false,
    backgroundColor: "#101412",
    title: "Motivo Studio",
    autoHideMenuBar: true,
    webPreferences: secureWebPreferences(preloadPath, app.isPackaged),
  });

  const trustedUrl = rendererUrl();
  installWindowSecurity(window, trustedUrl);
  installWindowZoom(window);
  window.once("ready-to-show", () => window.show());
  void window.loadURL(trustedUrl).then(() => {
    if (!window.isDestroyed() && !window.isVisible()) window.show();
  });
  return window;
}

function rendererUrl(): string {
  if (!app.isPackaged && MAIN_WINDOW_VITE_DEV_SERVER_URL) {
    const developmentUrl = new URL(MAIN_WINDOW_VITE_DEV_SERVER_URL);
    if (
      developmentUrl.protocol !== "http:" ||
      (developmentUrl.hostname !== "127.0.0.1" && developmentUrl.hostname !== "localhost")
    ) {
      throw new Error("The renderer development server must use a local HTTP endpoint.");
    }
    return developmentUrl.href;
  }
  return "motivo://app/index.html";
}
