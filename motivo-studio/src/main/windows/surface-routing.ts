import type { App, BrowserWindow, Event, IpcMain, IpcMainInvokeEvent } from "electron";
import type { IpcResult } from "../../shared/contracts";
import { emptyInputSchema, IPC } from "../../shared/ipc";
import {
  DEFAULT_STUDIO_SURFACE,
  studioSurfaceSchema,
  type StudioSurface,
} from "../../shared/surface";

export interface SurfaceRouting {
  current(): StudioSurface;
  dispose(): void;
}

interface SurfaceRoutingDependencies {
  readonly app: App;
  readonly ipcMain: IpcMain;
  readonly initialArgv: readonly string[];
  readonly getWindow: () => BrowserWindow | undefined;
}

export function requestedSurfaceFromArgv(argv: readonly string[]): StudioSurface | undefined {
  let requested: StudioSurface | undefined;

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    let candidate: string | undefined;
    if (argument === "--surface") {
      candidate = argv[index + 1];
      index += 1;
    } else if (argument?.startsWith("--surface=")) {
      candidate = argument.slice("--surface=".length);
    } else {
      continue;
    }

    const parsed = studioSurfaceSchema.safeParse(candidate);
    if (!parsed.success || requested !== undefined) return undefined;
    requested = parsed.data;
  }

  return requested;
}

export function initialSurfaceFromArgv(argv: readonly string[]): StudioSurface {
  return requestedSurfaceFromArgv(argv) ?? DEFAULT_STUDIO_SURFACE;
}

export function registerSurfaceRouting({
  app,
  ipcMain,
  initialArgv,
  getWindow,
}: SurfaceRoutingDependencies): SurfaceRouting {
  let currentSurface = initialSurfaceFromArgv(initialArgv);
  let disposed = false;

  const onSecondInstance = (_event: Event, argv: string[]): void => {
    const requested = requestedSurfaceFromArgv(argv);
    if (requested === undefined) return;
    currentSurface = requested;

    const window = getWindow();
    if (!window || window.isDestroyed()) return;
    if (!window.webContents.isDestroyed()) {
      window.webContents.send(IPC.surfaceChanged, requested);
    }
    if (window.isMinimized()) window.restore();
    if (!window.isVisible()) window.show();
    window.focus();
  };

  app.on("second-instance", onSecondInstance);
  ipcMain.handle(
    IPC.surfaceCurrent,
    (event: IpcMainInvokeEvent, raw: unknown): IpcResult<StudioSurface> => {
      const window = getWindow();
      if (!window || !trusted(event, window)) {
        return {
          ok: false,
          error: {
            code: "IPC_SOURCE_REJECTED",
            category: "validation",
            retryable: false,
            message: "The request did not originate from the Motivo application frame.",
          },
        };
      }
      if (!emptyInputSchema.safeParse(raw).success) {
        return {
          ok: false,
          error: {
            code: "IPC_INVALID_ARGUMENT",
            category: "validation",
            retryable: false,
            message: "The surface request did not match its bounded contract.",
          },
        };
      }
      return { ok: true, data: currentSurface };
    },
  );

  return {
    current: () => currentSurface,
    dispose: () => {
      if (disposed) return;
      disposed = true;
      app.removeListener("second-instance", onSecondInstance);
      ipcMain.removeHandler(IPC.surfaceCurrent);
    },
  };
}

function trusted(event: IpcMainInvokeEvent, window: BrowserWindow): boolean {
  return (
    event.sender.id === window.webContents.id &&
    event.senderFrame !== null &&
    event.senderFrame === window.webContents.mainFrame
  );
}
