import { contextBridge, ipcRenderer } from "electron";
import { z } from "zod";
import {
  actionStateSchema,
  ipcEnvelopeSchema,
  studioActionEventSchema,
  studioEventPageSchema,
  studioViewSchema,
  type MotivoBridge,
  type StudioError,
} from "../shared/contracts";
import { sessionListSchema, sessionViewSchema } from "../shared/session-contracts";
import {
  actionCancelInputSchema,
  actionStartInputSchema,
  emptyInputSchema,
  IPC,
  runEventsInputSchema,
  sessionAnswerInputSchema,
  sessionCurrentInputSchema,
  sessionListInputSchema,
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
  let actionListenerActive = false;
  const bridge: MotivoBridge = {
    studio: {
      current: () => invoke(IPC.studioCurrent, emptyInputSchema, studioViewSchema.nullable(), {}),
      openInitialized: () =>
        invoke(IPC.studioOpenInitialized, emptyInputSchema, studioViewSchema.nullable(), {}),
      initialize: () =>
        invoke(IPC.studioInitialize, emptyInputSchema, studioViewSchema.nullable(), {}),
      refresh: () => invoke(IPC.studioRefresh, emptyInputSchema, studioViewSchema, {}),
    },
    actions: {
      start: (input) => invoke(IPC.actionStart, actionStartInputSchema, actionStateSchema, input),
      cancel: async (input) => {
        await invoke(IPC.actionCancel, actionCancelInputSchema, z.null(), input);
      },
      subscribe: (listener) => {
        if (typeof listener !== "function")
          throw new TypeError("Action listener must be a function.");
        if (actionListenerActive) {
          throw new RangeError("Only one action listener may be active.");
        }
        const onEvent = (_event: unknown, raw: unknown) => {
          const parsed = studioActionEventSchema.safeParse(raw);
          if (parsed.success) listener(parsed.data);
        };
        ipcRenderer.on(IPC.actionEvent, onEvent);
        actionListenerActive = true;
        let active = true;
        return () => {
          if (!active) return;
          active = false;
          actionListenerActive = false;
          ipcRenderer.removeListener(IPC.actionEvent, onEvent);
        };
      },
    },
    runs: {
      events: (input) => invoke(IPC.runEvents, runEventsInputSchema, studioEventPageSchema, input),
    },
    sessions: {
      list: (input) => invoke(IPC.sessionList, sessionListInputSchema, sessionListSchema, input),
      current: (input) =>
        invoke(IPC.sessionCurrent, sessionCurrentInputSchema, sessionViewSchema, input),
      answer: (input) =>
        invoke(IPC.sessionAnswer, sessionAnswerInputSchema, sessionViewSchema, input),
    },
  };
  contextBridge.exposeInMainWorld("motivo", bridge);
}
