import { contextBridge, ipcRenderer } from "electron";
import { z } from "zod";
import {
  fileDocumentSchema,
  filePageSchema,
  ipcEnvelopeSchema,
  recoveryPageSchema,
  recoverySchema,
  runSchema,
  runStreamMessageSchema,
  schedulePageSchema,
  studioSnapshotSchema,
  subscriptionIdSchema,
  terminalProfileSchema,
  terminalSessionSchema,
  terminalStreamMessageSchema,
  workspaceSchema,
  type MotivoBridge,
  type RunStreamHandle,
  type StreamHandle,
  type StudioError,
} from "../shared/contracts";
import { studioSurfaceSchema } from "../shared/surface";
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
  runSubscribeRequestSchema,
  streamUnsubscribeInputSchema,
  terminalAckInputSchema,
  terminalCloseInputSchema,
  terminalCreateInputSchema,
  terminalResizeInputSchema,
  terminalSubscribeInputSchema,
  terminalSubscribeRequestSchema,
  terminalWriteInputSchema,
} from "../shared/ipc";

export class MotivoBridgeError extends Error {
  readonly detail: StudioError;

  constructor(detail: StudioError) {
    super(detail.message);
    this.name = "MotivoBridgeError";
    this.detail = detail;
  }
}

async function invoke<Input, Output>(
  channel: string,
  inputSchema: z.ZodType<Input>,
  outputSchema: z.ZodType<Output>,
  rawInput: unknown,
): Promise<Output> {
  const input = inputSchema.parse(rawInput);
  const raw: unknown = await ipcRenderer.invoke(channel, input);
  const envelope = ipcEnvelopeSchema.parse(raw);
  if (!envelope.ok) throw new MotivoBridgeError(envelope.error);
  return outputSchema.parse(envelope.data);
}

export function installBridge(): void {
  let surfaceListenerActive = false;
  const bridge: MotivoBridge = {
    surface: {
      current: () => invoke(IPC.surfaceCurrent, emptyInputSchema, studioSurfaceSchema, {}),
      subscribe: (listener) => {
        if (typeof listener !== "function") {
          throw new TypeError("Surface listener must be a function.");
        }
        if (surfaceListenerActive) {
          throw new RangeError("Only one surface listener may be active.");
        }
        const onSurface = (_event: unknown, raw: unknown) => {
          const surface = studioSurfaceSchema.safeParse(raw);
          if (surface.success) listener(surface.data);
        };
        ipcRenderer.on(IPC.surfaceChanged, onSurface);
        surfaceListenerActive = true;
        let active = true;
        return () => {
          if (!active) return;
          active = false;
          surfaceListenerActive = false;
          ipcRenderer.removeListener(IPC.surfaceChanged, onSurface);
        };
      },
    },
    system: {
      snapshot: () => invoke(IPC.systemSnapshot, emptyInputSchema, studioSnapshotSchema, {}),
    },
    workspaces: {
      open: () => invoke(IPC.workspaceOpen, emptyInputSchema, workspaceSchema.nullable(), {}),
    },
    files: {
      listPage: (input) => invoke(IPC.filesListPage, filePageInputSchema, filePageSchema, input),
      read: (input) => invoke(IPC.filesRead, fileReadInputSchema, fileDocumentSchema, input),
      save: (input) => invoke(IPC.filesSave, fileSaveInputSchema, fileDocumentSchema, input),
    },
    runs: {
      start: (input) => invoke(IPC.runsStart, runStartInputSchema, runSchema, input),
      get: (input) => invoke(IPC.runsGet, runGetInputSchema, runSchema, input),
      cancel: (input) => invoke(IPC.runsCancel, runCancelInputSchema, runSchema, input),
      subscribe: async (input, listener): Promise<RunStreamHandle> => {
        if (typeof listener !== "function") throw new TypeError("Run listener must be a function.");
        const request = runSubscribeRequestSchema.parse(input);
        const subscriptionId = subscriptionIdSchema.parse(crypto.randomUUID());
        const fullRequest = runSubscribeInputSchema.parse({ ...request, subscriptionId });
        const onMessage = (_event: unknown, raw: unknown) => {
          const message = runStreamMessageSchema.safeParse(raw);
          if (message.success && message.data.subscriptionId === subscriptionId) {
            listener(message.data);
          }
        };
        ipcRenderer.on(IPC.runsEvent, onMessage);
        try {
          await invoke(IPC.runsSubscribe, runSubscribeInputSchema, z.undefined(), fullRequest);
        } catch (error) {
          ipcRenderer.removeListener(IPC.runsEvent, onMessage);
          throw error;
        }
        let active = true;
        return {
          subscriptionId,
          ack: (highestSequence) =>
            invoke(IPC.runsAck, runAckInputSchema, z.undefined(), {
              subscriptionId,
              highestSequence,
            }),
          unsubscribe: async () => {
            if (!active) return;
            active = false;
            ipcRenderer.removeListener(IPC.runsEvent, onMessage);
            await invoke(IPC.runsUnsubscribe, streamUnsubscribeInputSchema, z.undefined(), {
              subscriptionId,
            });
          },
        };
      },
    },
    schedules: {
      listPage: (input) =>
        invoke(IPC.schedulesListPage, pageInputSchema, schedulePageSchema, input),
    },
    recovery: {
      listPage: (input) =>
        invoke(IPC.recoveryListPage, recoveryPageInputSchema, recoveryPageSchema, input),
      apply: (input) => invoke(IPC.recoveryApply, recoveryApplyInputSchema, recoverySchema, input),
    },
    terminals: {
      profiles: () =>
        invoke(IPC.terminalsProfiles, emptyInputSchema, z.array(terminalProfileSchema).max(8), {}),
      create: (input) =>
        invoke(IPC.terminalsCreate, terminalCreateInputSchema, terminalSessionSchema, input),
      write: (input) => invoke(IPC.terminalsWrite, terminalWriteInputSchema, z.undefined(), input),
      resize: (input) =>
        invoke(IPC.terminalsResize, terminalResizeInputSchema, z.undefined(), input),
      ack: (input) => invoke(IPC.terminalsAck, terminalAckInputSchema, z.undefined(), input),
      close: (input) => invoke(IPC.terminalsClose, terminalCloseInputSchema, z.undefined(), input),
      subscribe: async (input, listener): Promise<StreamHandle> => {
        if (typeof listener !== "function") {
          throw new TypeError("Terminal listener must be a function.");
        }
        const request = terminalSubscribeRequestSchema.parse(input);
        const subscriptionId = subscriptionIdSchema.parse(crypto.randomUUID());
        const fullRequest = terminalSubscribeInputSchema.parse({ ...request, subscriptionId });
        const onMessage = (_event: unknown, raw: unknown) => {
          const message = terminalStreamMessageSchema.safeParse(raw);
          if (message.success && message.data.terminalId === request.terminalId) {
            listener(message.data);
          }
        };
        ipcRenderer.on(IPC.terminalsEvent, onMessage);
        try {
          await invoke(
            IPC.terminalsSubscribe,
            terminalSubscribeInputSchema,
            z.undefined(),
            fullRequest,
          );
        } catch (error) {
          ipcRenderer.removeListener(IPC.terminalsEvent, onMessage);
          throw error;
        }
        let active = true;
        return {
          subscriptionId,
          unsubscribe: async () => {
            if (!active) return;
            active = false;
            ipcRenderer.removeListener(IPC.terminalsEvent, onMessage);
            await invoke(IPC.terminalsUnsubscribe, streamUnsubscribeInputSchema, z.undefined(), {
              subscriptionId,
            });
          },
        };
      },
    },
  };

  contextBridge.exposeInMainWorld("motivo", bridge);
}
