import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  entryIdSchema,
  recoveryIdSchema,
  runIdSchema,
  scheduleIdSchema,
  sequenceSchema,
  subscriptionIdSchema,
  workspaceIdSchema,
  type MotivoBridge,
  type RunStreamHandle,
  type RunStreamMessage,
} from "../shared/contracts";
import type { StudioSurface } from "../shared/surface";
import App from "./App";

vi.mock("./editor/EditorPane", () => ({
  EditorPane: ({ path }: { path: string }) => <div aria-label={`Editor for ${path}`} />,
}));
vi.mock("./components/TerminalPane", () => ({
  TerminalPane: () => <div aria-label="Terminal">Terminal</div>,
}));

const workspaceId = workspaceIdSchema.parse("workspace-1");
const rootEntryId = entryIdSchema.parse("entry-root");
const fileEntryId = entryIdSchema.parse("entry-main");

interface RunStreamRecorder {
  readonly requests: unknown[];
  readonly handles: RunStreamHandle[];
  readonly listeners: ((message: RunStreamMessage) => void)[];
}

describe("Motivo renderer vertical slice", () => {
  let bridge: MotivoBridge;
  let recorder: RunStreamRecorder;

  beforeEach(() => {
    ({ bridge, recorder } = fakeBridge());
    Object.defineProperty(window, "motivo", { configurable: true, value: bridge });
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  afterEach(() => {
    Reflect.deleteProperty(window, "motivo");
  });

  it("opens a paged workspace, reads a file, starts a run, and exposes scheduler/recovery views", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("LOCAL CONTROL PLANE")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Open workspace" }));
    expect(await screen.findByRole("button", { name: /main\.py/ })).toBeVisible();
    await user.click(screen.getByRole("button", { name: /main\.py/ }));
    expect(await screen.findByLabelText("Editor for main.py")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Run file" }));
    await waitFor(() => expect(bridge.runs.start).toHaveBeenCalledOnce());
    expect(await screen.findByText("queued")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Scheduler" }));
    expect(await screen.findByText("Nightly validation")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Recovery" }));
    expect(await screen.findByText("Before failed run")).toBeVisible();
  });

  it("opens directly on the scheduler requested by the typed startup surface", async () => {
    const unsubscribe = vi.fn();
    vi.mocked(bridge.surface.current).mockResolvedValueOnce("scheduler");
    vi.mocked(bridge.surface.subscribe).mockReturnValueOnce(unsubscribe);

    const view = render(<App />);

    expect(await screen.findByRole("region", { name: "Scheduler view" })).toBeVisible();
    expect(await screen.findByText("Nightly validation")).toBeVisible();
    view.unmount();
    expect(unsubscribe).toHaveBeenCalledOnce();
  });

  it("switches an existing renderer to scheduler and unregisters the surface listener", async () => {
    let route: ((surface: StudioSurface) => void) | undefined;
    const unsubscribe = vi.fn();
    vi.mocked(bridge.surface.subscribe).mockImplementationOnce((listener) => {
      route = listener;
      return unsubscribe;
    });

    const view = render(<App />);
    expect(await screen.findByRole("region", { name: "Paged workspace files" })).toBeVisible();
    const routeToSurface = route;
    if (!routeToSurface) throw new Error("expected the surface route listener");
    act(() => routeToSurface("scheduler"));

    expect(await screen.findByRole("region", { name: "Scheduler view" })).toBeVisible();
    view.unmount();
    expect(unsubscribe).toHaveBeenCalledOnce();
  });

  it("unsubscribes the prior run stream when switching workspaces and rejects stale events", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("LOCAL CONTROL PLANE");

    await user.click(screen.getByRole("button", { name: "Open workspace" }));
    await user.click(screen.getByRole("button", { name: /main\.py/ }));
    await user.click(screen.getByRole("button", { name: "Run file" }));
    await waitFor(() => expect(bridge.runs.subscribe).toHaveBeenCalledOnce());

    const firstHandle = recorder.handles[0];
    if (!firstHandle) throw new Error("expected a captured run stream handle");
    const firstListener = recorder.listeners[0];
    if (!firstListener) throw new Error("expected a captured run stream listener");

    vi.mocked(bridge.workspaces.open).mockResolvedValueOnce({
      id: workspaceIdSchema.parse("workspace-2"),
      name: "motivo-second",
      revision: "revision-2",
      rootEntryId: entryIdSchema.parse("entry-root-2"),
    });
    await user.click(screen.getByRole("button", { name: "Open workspace" }));

    expect(await screen.findByText("revision revision-2")).toBeVisible();
    await waitFor(() => expect(firstHandle.unsubscribe).toHaveBeenCalledOnce());
    expect(bridge.runs.subscribe).toHaveBeenCalledOnce();
    expect(screen.getByText("idle")).toBeVisible();

    firstListener({
      kind: "events",
      subscriptionId: subscriptionIdSchema.parse("f61ab537-889e-4aa2-b218-4b9a12becbd2"),
      events: [
        {
          runId: runIdSchema.parse("run-1"),
          sequence: sequenceSchema.parse("1"),
          occurredAt: "2026-08-01T12:00:01.000Z",
          body: { kind: "output", stream: "stdout", data: "STALE-OLD-WORKSPACE", truncated: false },
        },
      ],
    });
    expect(screen.queryByText("STALE-OLD-WORKSPACE")).toBeNull();
  });

  it("keeps the active workspace, run, and subscription when the open dialog is cancelled", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("LOCAL CONTROL PLANE");

    await user.click(screen.getByRole("button", { name: "Open workspace" }));
    await user.click(screen.getByRole("button", { name: /main\.py/ }));
    await user.click(screen.getByRole("button", { name: "Run file" }));
    await waitFor(() => expect(bridge.runs.subscribe).toHaveBeenCalledOnce());
    const firstHandle = recorder.handles[0];
    if (!firstHandle) throw new Error("expected a captured run stream handle");

    expect(screen.getByText("revision revision-1")).toBeVisible();
    expect(screen.getByText("queued")).toBeVisible();

    vi.mocked(bridge.workspaces.open).mockResolvedValueOnce(null);
    await user.click(screen.getByRole("button", { name: "Open workspace" }));
    await waitFor(() => expect(bridge.workspaces.open).toHaveBeenCalledTimes(2));

    expect(screen.getByText("revision revision-1")).toBeVisible();
    expect(screen.queryByText("revision revision-2")).toBeNull();
    expect(screen.getByText("queued")).toBeVisible();
    expect(bridge.runs.subscribe).toHaveBeenCalledOnce();
    expect(firstHandle.unsubscribe).not.toHaveBeenCalled();
  });
});

function fakeBridge(): { bridge: MotivoBridge; recorder: RunStreamRecorder } {
  const recorder: RunStreamRecorder = { requests: [], handles: [], listeners: [] };
  const workspace = {
    id: workspaceId,
    name: "motivo-demo",
    revision: "revision-1",
    rootEntryId,
  };
  const bridge: MotivoBridge = {
    surface: {
      current: vi.fn().mockResolvedValue("files"),
      subscribe: vi.fn().mockReturnValue(vi.fn()),
    },
    system: {
      snapshot: vi.fn().mockResolvedValue({
        state: "ready",
        version: "0.1.0",
        services: [
          { service: "agentrod", state: "ready", instanceId: "agentrod-1" },
          { service: "tactusd", state: "ready", instanceId: "tactusd-1" },
          { service: "segnod", state: "ready", instanceId: "segnod-1" },
        ],
      }),
    },
    workspaces: { open: vi.fn().mockResolvedValue(workspace) },
    files: {
      listPage: vi.fn().mockResolvedValue({
        workspaceId,
        parentId: rootEntryId,
        entries: [
          {
            id: fileEntryId,
            parentId: rootEntryId,
            name: "main.py",
            kind: "file",
            sizeBytes: 9,
            language: "python",
            revision: "file-revision-1",
            readOnly: false,
          },
        ],
      }),
      read: vi.fn().mockResolvedValue({
        workspaceId,
        entryId: fileEntryId,
        name: "main.py",
        content: "print(1)",
        revision: "file-revision-1",
        language: "python",
        readOnly: false,
        binary: false,
        truncated: false,
      }),
      save: vi.fn(),
    },
    runs: {
      start: vi.fn().mockResolvedValue({
        id: runIdSchema.parse("run-1"),
        workspaceId,
        state: "queued",
        lastSequence: sequenceSchema.parse("0"),
        updatedAt: "2026-08-01T12:00:00.000Z",
      }),
      get: vi.fn(),
      cancel: vi.fn(),
      subscribe: vi
        .fn()
        .mockImplementation((request: unknown, listener: (message: RunStreamMessage) => void) => {
          recorder.requests.push(request);
          recorder.listeners.push(listener);
          const handle: RunStreamHandle = {
            subscriptionId: subscriptionIdSchema.parse("f61ab537-889e-4aa2-b218-4b9a12becbd2"),
            ack: vi.fn().mockResolvedValue(undefined),
            unsubscribe: vi.fn().mockResolvedValue(undefined),
          };
          recorder.handles.push(handle);
          return Promise.resolve(handle);
        }),
    },
    schedules: {
      listPage: vi.fn().mockResolvedValue({
        schedules: [
          {
            id: scheduleIdSchema.parse("schedule-1"),
            taskId: "task-1",
            label: "Nightly validation",
            cron: "0 2 * * *",
            timezone: "UTC",
            state: "ready",
            nextFireAt: "2026-08-02T02:00:00.000Z",
          },
        ],
      }),
    },
    recovery: {
      listPage: vi.fn().mockResolvedValue({
        records: [
          {
            id: recoveryIdSchema.parse("recovery-1"),
            workspaceId,
            label: "Before failed run",
            state: "available",
            createdAt: "2026-08-01T11:00:00.000Z",
            changedFiles: 2,
          },
        ],
      }),
      apply: vi.fn(),
    },
    terminals: {
      profiles: vi.fn().mockResolvedValue([]),
      create: vi.fn(),
      write: vi.fn(),
      resize: vi.fn(),
      ack: vi.fn(),
      close: vi.fn(),
      subscribe: vi.fn(),
    },
  };
  return { bridge, recorder };
}
