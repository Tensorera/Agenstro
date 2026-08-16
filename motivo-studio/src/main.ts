import { join } from "node:path";
import { app, dialog, ipcMain, Menu, type BrowserWindow } from "electron";
import { asStudioError } from "./main/errors";
import { registerIpcHandlers } from "./main/ipc/handlers";
import {
  launchData,
  parseLaunchData,
  parseLaunchRequest,
  WorkspaceOpenQueue,
  type LaunchRequest,
} from "./main/launch";
import { TactusController } from "./main/tactus/controller";
import { installAppProtocol, registerAppScheme } from "./main/windows/app-protocol";
import { createMainWindow } from "./main/windows/main-window";
import { IPC } from "./shared/ipc";

let mainWindow: BrowserWindow | undefined;
let unregisterIpc: (() => void) | undefined;
let workspaceOpener: WorkspaceOpenQueue<unknown> | undefined;
const pendingLaunches: LaunchRequest[] = [];

let initialLaunch: LaunchRequest = {};
let launchArgumentError: unknown;
try {
  initialLaunch = parseLaunchRequest(process.argv, process.cwd(), app.isPackaged);
} catch (error) {
  launchArgumentError = error;
}

registerAppScheme();

if (!app.requestSingleInstanceLock(launchData(initialLaunch))) {
  app.quit();
} else {
  app.on("second-instance", (_event, _argv, _workingDirectory, additionalData) => {
    acceptLaunch(parseLaunchData(additionalData));
  });
  void app.whenReady().then(() => startApplication(initialLaunch));
}

function startApplication(request: LaunchRequest): void {
  Menu.setApplicationMenu(null);
  if (app.isPackaged) installAppProtocol(join(__dirname, "../renderer/main_window"));

  const window = createMainWindow();
  mainWindow = window;
  const controller = new TactusController({
    emit: (event) => {
      if (!window.isDestroyed() && !window.webContents.isDestroyed()) {
        window.webContents.send(IPC.actionEvent, event);
      }
    },
  });
  const opener = new WorkspaceOpenQueue((root) => controller.open(root));
  workspaceOpener = opener;
  let initialError: unknown;
  const initialReady = request.workspacePath
    ? opener.open(request.workspacePath).then(
        () => undefined,
        (error: unknown) => {
          initialError = error;
        },
      )
    : Promise.resolve();
  unregisterIpc = registerIpcHandlers({
    ipcMain,
    window,
    dialog,
    controller,
    openWorkspace: (root) => opener.open(root),
    initialReady,
  });
  void initialReady.then(() => {
    if (initialError) showLaunchError(initialError);
  });
  if (launchArgumentError) showLaunchError(launchArgumentError);
  for (const pending of pendingLaunches.splice(0)) acceptLaunch(pending);
  window.on("closed", () => {
    if (mainWindow === window) mainWindow = undefined;
    workspaceOpener = undefined;
    unregisterIpc?.();
    unregisterIpc = undefined;
  });
}

function acceptLaunch(request: LaunchRequest): void {
  const window = mainWindow;
  const opener = workspaceOpener;
  if (!window || !opener) {
    pendingLaunches.push(request);
    return;
  }
  if (!request.workspacePath) {
    focusWindow(window);
    return;
  }
  void opener.open(request.workspacePath).then(
    () => {
      if (!window.isDestroyed()) window.webContents.reload();
      focusWindow(window);
    },
    (error: unknown) => {
      showLaunchError(error);
      focusWindow(window);
    },
  );
}

function focusWindow(window: BrowserWindow): void {
  if (window.isDestroyed()) return;
  if (window.isMinimized()) window.restore();
  window.show();
  window.focus();
}

function showLaunchError(error: unknown): void {
  const usageMessage =
    error instanceof Error && error.message.startsWith("Usage: motivo-studio")
      ? error.message
      : undefined;
  const message = usageMessage ?? asStudioError(error).message;
  dialog.showErrorBox("Motivo Studio could not open the workspace", message);
}

app.on("window-all-closed", () => app.quit());
app.on("before-quit", () => {
  unregisterIpc?.();
  unregisterIpc = undefined;
});
