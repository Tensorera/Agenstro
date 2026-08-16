import { join } from "node:path";
import { app, dialog, ipcMain, Menu, type BrowserWindow } from "electron";
import type { BootstrappedDaemons } from "./main/daemon/bootstrap";
import { startBundledDaemons } from "./main/daemon/bootstrap";
import type { DaemonClient } from "./main/daemon/daemon-client";
import { GrpcDaemonClient } from "./main/daemon/grpc-daemon-client";
import { UnavailableDaemonClient } from "./main/daemon/unavailable-client";
import { registerIpcHandlers } from "./main/ipc/handlers";
import type { PtyBroker } from "./main/pty/pty-broker";
import { UtilityProcessPtyBroker } from "./main/pty/pty-broker";
import { installAppProtocol, registerAppScheme } from "./main/windows/app-protocol";
import { createMainWindow } from "./main/windows/main-window";
import { registerSurfaceRouting, type SurfaceRouting } from "./main/windows/surface-routing";

let daemons: BootstrappedDaemons | undefined;
let daemonClient: DaemonClient = new UnavailableDaemonClient();
let ptyBroker: PtyBroker | undefined;
let unregisterIpc: (() => void) | undefined;
let mainWindow: BrowserWindow | undefined;
let surfaceRouting: SurfaceRouting | undefined;
let cleanupComplete = false;
let cleanupStarted: Promise<void> | undefined;

registerAppScheme();

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  surfaceRouting = registerSurfaceRouting({
    app,
    ipcMain,
    initialArgv: process.argv,
    getWindow: () => mainWindow,
  });
  void app.whenReady().then(startApplication);
}

async function startApplication(): Promise<void> {
  Menu.setApplicationMenu(null);
  if (app.isPackaged) {
    installAppProtocol(join(__dirname, "../renderer/main_window"));
  }
  try {
    daemons = await startBundledDaemons(process.resourcesPath);
    daemonClient = await GrpcDaemonClient.connect(daemons.connections);
  } catch {
    await daemons?.close();
    daemons = undefined;
    daemonClient = new UnavailableDaemonClient();
  }

  const nodePtyRoot = app.isPackaged
    ? join(process.resourcesPath, "node-pty")
    : join(process.cwd(), "node_modules", "node-pty");
  ptyBroker = new UtilityProcessPtyBroker(join(__dirname, "pty-host.js"), nodePtyRoot);
  const window = createMainWindow();
  mainWindow = window;
  unregisterIpc = registerIpcHandlers({
    ipcMain,
    window,
    dialog,
    daemon: daemonClient,
    pty: ptyBroker,
  });

  window.on("closed", () => {
    if (mainWindow === window) mainWindow = undefined;
    unregisterIpc?.();
    unregisterIpc = undefined;
  });
}

app.on("window-all-closed", () => app.quit());
app.on("before-quit", (event) => {
  if (cleanupComplete) return;
  event.preventDefault();
  cleanupStarted ??= cleanup().finally(() => {
    cleanupComplete = true;
    app.quit();
  });
});

async function cleanup(): Promise<void> {
  surfaceRouting?.dispose();
  surfaceRouting = undefined;
  unregisterIpc?.();
  unregisterIpc = undefined;
  await ptyBroker?.shutdown();
  ptyBroker = undefined;
  daemonClient.close();
  await daemons?.close();
  daemons = undefined;
}
