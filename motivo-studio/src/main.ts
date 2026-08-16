import { join } from "node:path";
import { app, dialog, ipcMain, Menu, type BrowserWindow } from "electron";
import { registerIpcHandlers } from "./main/ipc/handlers";
import { installAppProtocol, registerAppScheme } from "./main/windows/app-protocol";
import { createMainWindow } from "./main/windows/main-window";

let mainWindow: BrowserWindow | undefined;
let unregisterIpc: (() => void) | undefined;

registerAppScheme();

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on("second-instance", () => {
    if (!mainWindow) return;
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.focus();
  });
  void app.whenReady().then(startApplication);
}

function startApplication(): void {
  Menu.setApplicationMenu(null);
  if (app.isPackaged) installAppProtocol(join(__dirname, "../renderer/main_window"));

  const window = createMainWindow();
  mainWindow = window;
  unregisterIpc = registerIpcHandlers({ ipcMain, window, dialog });
  window.on("closed", () => {
    if (mainWindow === window) mainWindow = undefined;
    unregisterIpc?.();
    unregisterIpc = undefined;
  });
}

app.on("window-all-closed", () => app.quit());
app.on("before-quit", () => {
  unregisterIpc?.();
  unregisterIpc = undefined;
});
