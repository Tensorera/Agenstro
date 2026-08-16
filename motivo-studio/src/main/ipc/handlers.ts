import type { BrowserWindow, Dialog, IpcMain, IpcMainInvokeEvent } from "electron";
import { z } from "zod";
import {
  actionStateSchema,
  ipcEnvelopeSchema,
  studioEventPageSchema,
  studioViewSchema,
  type ActionRequest,
  type ActionState,
  type IpcResult,
  type StudioActionEvent,
  type StudioEventPage,
  type StudioView,
} from "../../shared/contracts";
import {
  actionCancelInputSchema,
  actionStartInputSchema,
  emptyInputSchema,
  IPC,
  runEventsInputSchema,
} from "../../shared/ipc";
import { asStudioError } from "../errors";
import { TactusController } from "../tactus/controller";

interface HandlerDependencies {
  readonly ipcMain: IpcMain;
  readonly window: BrowserWindow;
  readonly dialog: Pick<Dialog, "showOpenDialog">;
  readonly tactusExecutable?: string;
  readonly controller?: StudioController;
  readonly openWorkspace?: (root: string) => Promise<StudioView>;
  readonly initialReady?: Promise<void>;
}

export interface StudioController {
  current(): StudioView | null;
  open(root: string): Promise<StudioView>;
  initialize(root: string): Promise<StudioView>;
  refresh(): Promise<StudioView>;
  events(runId: string, after: string, limit?: number): Promise<StudioEventPage>;
  start(input: ActionRequest): ActionState;
  cancel(actionId: string): void;
  dispose(): void;
}

export function registerIpcHandlers(dependencies: HandlerDependencies): () => void {
  const { ipcMain, window, dialog } = dependencies;
  const controller =
    dependencies.controller ??
    new TactusController({
      ...(dependencies.tactusExecutable ? { executable: dependencies.tactusExecutable } : {}),
      emit: (event) => sendActionEvent(window, event),
    });
  const openWorkspace = dependencies.openWorkspace ?? ((root: string) => controller.open(root));
  const initialReady = dependencies.initialReady ?? Promise.resolve();
  const channels: string[] = [];

  const register = <Input, Output>(
    channel: string,
    inputSchema: z.ZodType<Input>,
    outputSchema: z.ZodType<Output>,
    operation: (input: Input) => Output | Promise<Output>,
  ): void => {
    channels.push(channel);
    ipcMain.handle(channel, async (event: IpcMainInvokeEvent, raw: unknown) => {
      if (
        event.sender !== window.webContents ||
        event.senderFrame !== window.webContents.mainFrame
      ) {
        return failure(new Error("IPC sender is not the Motivo Studio window."));
      }
      try {
        const input = inputSchema.parse(raw);
        const output = outputSchema.parse(await operation(input));
        return { ok: true, data: output } satisfies IpcResult<Output>;
      } catch (error) {
        return failure(error);
      }
    });
  };

  register(IPC.studioCurrent, emptyInputSchema, studioViewSchema.nullable(), async () => {
    await initialReady;
    return controller.current();
  });
  register(IPC.studioOpenInitialized, emptyInputSchema, studioViewSchema.nullable(), async () => {
    const folder = await selectFolder(dialog, window, "Open initialized Tactus workspace");
    await initialReady.catch(() => undefined);
    return folder ? openWorkspace(folder) : null;
  });
  register(IPC.studioInitialize, emptyInputSchema, studioViewSchema.nullable(), async () => {
    const folder = await selectFolder(dialog, window, "Initialize folder with Tactus");
    return folder ? controller.initialize(folder) : null;
  });
  register(IPC.studioRefresh, emptyInputSchema, studioViewSchema, () => controller.refresh());
  register(IPC.actionStart, actionStartInputSchema, actionStateSchema, (input) =>
    controller.start(input),
  );
  register(IPC.actionCancel, actionCancelInputSchema, z.null(), (input) => {
    controller.cancel(input.actionId);
    return null;
  });
  register(IPC.runEvents, runEventsInputSchema, studioEventPageSchema, (input) =>
    controller.events(input.runId, input.after, input.limit),
  );

  return () => {
    for (const channel of channels) ipcMain.removeHandler(channel);
    controller.dispose();
  };
}

async function selectFolder(
  dialog: Pick<Dialog, "showOpenDialog">,
  window: BrowserWindow,
  title: string,
): Promise<string | null> {
  const response = await dialog.showOpenDialog(window, {
    title,
    properties: ["openDirectory"],
  });
  return response.canceled ? null : (response.filePaths[0] ?? null);
}

function sendActionEvent(window: BrowserWindow, event: StudioActionEvent): void {
  if (!window.isDestroyed() && !window.webContents.isDestroyed()) {
    window.webContents.send(IPC.actionEvent, event);
  }
}

function failure(error: unknown): IpcResult<never> {
  const envelope = { ok: false, error: asStudioError(error) } as const;
  return ipcEnvelopeSchema.parse(envelope) as IpcResult<never>;
}
