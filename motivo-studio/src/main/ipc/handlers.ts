import { randomUUID } from "node:crypto";
import type { BrowserWindow, IpcMain, IpcMainInvokeEvent, OpenDialogOptions } from "electron";
import { z } from "zod";
import {
  recoveryPageSchema,
  recoverySchema,
  requestIdSchema,
  runSchema,
  schedulePageSchema,
  studioSnapshotSchema,
  terminalProfileSchema,
  terminalSessionSchema,
  workspaceSchema,
  fileDocumentSchema,
  filePageSchema,
  type IpcResult,
} from "../../shared/contracts";
import {
  emptyInputSchema,
  filePageInputSchema,
  fileReadInputSchema,
  fileSaveInputSchema,
  IPC,
  pageInputSchema,
  recoveryApplyInputSchema,
  recoveryPageInputSchema,
  runAckInputSchema,
  runCancelInputSchema,
  runGetInputSchema,
  runStartInputSchema,
  runSubscribeInputSchema,
  streamUnsubscribeInputSchema,
  terminalAckInputSchema,
  terminalCloseInputSchema,
  terminalCreateInputSchema,
  terminalResizeInputSchema,
  terminalSubscribeInputSchema,
  terminalWriteInputSchema,
} from "../../shared/ipc";
import type { DaemonClient } from "../daemon/daemon-client";
import { normalizeFault, StudioFault } from "../errors";
import type { PtyBroker } from "../pty/pty-broker";
import { RunSubscriptionManager } from "./run-subscriptions";
import { TerminalSubscriptionManager } from "./terminal-subscriptions";

interface DialogPort {
  showOpenDialog(
    window: BrowserWindow,
    options: OpenDialogOptions,
  ): Promise<{ readonly canceled: boolean; readonly filePaths: string[] }>;
}

export interface IpcDependencies {
  readonly ipcMain: IpcMain;
  readonly window: BrowserWindow;
  readonly dialog: DialogPort;
  readonly daemon: DaemonClient;
  readonly pty: PtyBroker;
}

export function registerIpcHandlers(dependencies: IpcDependencies): () => void {
  const { ipcMain, window, dialog, daemon, pty } = dependencies;
  const runSubscriptions = new RunSubscriptionManager(daemon);
  const terminalSubscriptions = new TerminalSubscriptionManager(pty);
  const channels: string[] = [];

  const register = <Input, Output>(
    channel: string,
    inputSchema: z.ZodType<Input>,
    outputSchema: z.ZodType<Output>,
    handler: (input: Input, event: IpcMainInvokeEvent) => Output | Promise<Output>,
  ) => {
    channels.push(channel);
    ipcMain.handle(channel, async (event, raw: unknown): Promise<IpcResult<Output>> => {
      if (!trusted(event, window)) {
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
      try {
        const input = inputSchema.parse(raw);
        const output = await handler(input, event);
        return { ok: true, data: outputSchema.parse(output) };
      } catch (error) {
        if (error instanceof z.ZodError) {
          return {
            ok: false,
            error: {
              code: "IPC_INVALID_ARGUMENT",
              category: "validation",
              retryable: false,
              message: "The IPC request or response did not match its bounded contract.",
            },
          };
        }
        return { ok: false, error: normalizeFault(error) };
      }
    });
  };

  register(IPC.systemSnapshot, emptyInputSchema, studioSnapshotSchema, () => daemon.snapshot());
  register(IPC.workspaceOpen, emptyInputSchema, workspaceSchema.nullable(), async () => {
    const result = await dialog.showOpenDialog(window, {
      title: "Open a Motivo workspace",
      buttonLabel: "Open workspace",
      properties: ["openDirectory"],
    });
    const selected = result.filePaths[0];
    if (result.canceled || selected === undefined) return null;
    const opened = await daemon.openWorkspace(selected, requestIdSchema.parse(randomUUID()));
    return opened.workspace;
  });
  register(IPC.filesListPage, filePageInputSchema, filePageSchema, (input) =>
    daemon.listFiles(input),
  );
  register(IPC.filesRead, fileReadInputSchema, fileDocumentSchema, (input) =>
    daemon.readFile(input),
  );
  register(IPC.filesSave, fileSaveInputSchema, fileDocumentSchema, (input) =>
    daemon.saveFile(input),
  );
  register(IPC.runsStart, runStartInputSchema, runSchema, (input) => daemon.startRun(input));
  register(IPC.runsGet, runGetInputSchema, runSchema, (input) => daemon.getRun(input));
  register(IPC.runsCancel, runCancelInputSchema, runSchema, (input) => daemon.cancelRun(input));
  register(IPC.runsSubscribe, runSubscribeInputSchema, z.undefined(), (input, event) => {
    runSubscriptions.subscribe(event.sender, input);
  });
  register(IPC.runsAck, runAckInputSchema, z.undefined(), (input, event) => {
    runSubscriptions.acknowledge(event.sender.id, input.subscriptionId, input.highestSequence);
  });
  register(IPC.runsUnsubscribe, streamUnsubscribeInputSchema, z.undefined(), (input, event) => {
    runSubscriptions.unsubscribe(event.sender.id, input.subscriptionId);
  });
  register(IPC.schedulesListPage, pageInputSchema, schedulePageSchema, (input) =>
    daemon.listSchedules(input),
  );
  register(IPC.recoveryListPage, recoveryPageInputSchema, recoveryPageSchema, (input) =>
    daemon.listRecoveries(input),
  );
  register(IPC.recoveryApply, recoveryApplyInputSchema, recoverySchema, (input) =>
    daemon.recover(input),
  );
  register(IPC.terminalsProfiles, emptyInputSchema, z.array(terminalProfileSchema).max(8), () => [
    ...pty.profiles(),
  ]);
  register(
    IPC.terminalsCreate,
    terminalCreateInputSchema,
    terminalSessionSchema,
    async (input, event) => {
      const session = await pty.create({
        cwd: daemon.terminalCwd(input.workspaceId),
        profileId: input.profileId,
        cols: input.cols,
        rows: input.rows,
      });
      terminalSubscriptions.register(event.sender, session.id);
      return session;
    },
  );
  register(IPC.terminalsWrite, terminalWriteInputSchema, z.undefined(), (input, event) => {
    assertTerminalOwner(terminalSubscriptions, event.sender.id, input.terminalId);
    pty.write(input.terminalId, input.data);
  });
  register(IPC.terminalsResize, terminalResizeInputSchema, z.undefined(), (input, event) => {
    assertTerminalOwner(terminalSubscriptions, event.sender.id, input.terminalId);
    pty.resize(input.terminalId, input.cols, input.rows);
  });
  register(IPC.terminalsAck, terminalAckInputSchema, z.undefined(), (input, event) => {
    terminalSubscriptions.acknowledge(event.sender.id, input.terminalId, input.highestSequence);
  });
  register(IPC.terminalsClose, terminalCloseInputSchema, z.undefined(), (input, event) => {
    terminalSubscriptions.close(event.sender.id, input.terminalId);
  });
  register(IPC.terminalsSubscribe, terminalSubscribeInputSchema, z.undefined(), (input, event) => {
    terminalSubscriptions.subscribe(event.sender, input.subscriptionId, input.terminalId);
  });
  register(
    IPC.terminalsUnsubscribe,
    streamUnsubscribeInputSchema,
    z.undefined(),
    (input, event) => {
      terminalSubscriptions.unsubscribe(event.sender.id, input.subscriptionId);
    },
  );

  const closeWindow = () => {
    runSubscriptions.closeOwner(window.webContents.id);
    terminalSubscriptions.closeOwner(window.webContents.id);
  };
  window.webContents.once("destroyed", closeWindow);

  return () => {
    window.webContents.removeListener("destroyed", closeWindow);
    runSubscriptions.closeAll();
    terminalSubscriptions.shutdown();
    channels.forEach((channel) => ipcMain.removeHandler(channel));
  };
}

function trusted(event: IpcMainInvokeEvent, window: BrowserWindow): boolean {
  return (
    event.sender.id === window.webContents.id &&
    event.senderFrame !== null &&
    event.senderFrame === window.webContents.mainFrame
  );
}

function assertTerminalOwner(
  manager: TerminalSubscriptionManager,
  ownerId: number,
  terminalId: Parameters<TerminalSubscriptionManager["close"]>[1],
): void {
  try {
    manager.assertOwner(ownerId, terminalId);
  } catch {
    throw new StudioFault({
      code: "TERMINAL_NOT_FOUND",
      category: "resource",
      retryable: false,
      message: "The terminal is not owned by this window.",
    });
  }
}
