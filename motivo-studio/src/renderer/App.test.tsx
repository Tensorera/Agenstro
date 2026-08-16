import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  MotivoBridge,
  StudioActionEvent,
  StudioEventPage,
  StudioView,
} from "../shared/contracts";
import App from "./App";

const actionId = "3ce53087-2218-42fd-bdda-afc4097020ae";
const handle = "aa665bbe-ece0-40e6-8235-2278635aee84";

describe("Motivo Studio renderer", () => {
  let bridge: MotivoBridge;
  let publish: (event: StudioActionEvent) => void;

  beforeEach(() => {
    ({ bridge, publish } = fakeBridge());
    Object.defineProperty(window, "motivo", { configurable: true, value: bridge });
  });

  afterEach(() => {
    Reflect.deleteProperty(window, "motivo");
  });

  it("offers both workspace entry paths without exposing a host path", async () => {
    vi.mocked(bridge.studio.current).mockResolvedValueOnce(null);
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole("region", { name: "No workspace" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Open workspace" }));

    expect(bridge.studio.openInitialized).toHaveBeenCalledOnce();
    expect(await screen.findByText("topology-demo")).toBeVisible();
    expect(document.body.textContent).not.toContain("D:\\");
  });

  it("generates with the selected provider, streams output, and refreshes after completion", async () => {
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("Workflow entries")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Workflow" }));
    await user.type(screen.getByLabelText("Workflow goal"), "Build a typed two-step analyzer");
    await user.click(screen.getByRole("button", { name: "Generate workflow" }));

    expect(bridge.actions.start).toHaveBeenCalledWith({
      kind: "generate",
      goal: "Build a typed two-step analyzer",
      provider: "codex",
    });
    expect(await screen.findByRole("region", { name: "Current action output" })).toBeVisible();

    act(() => {
      publish({
        type: "output",
        actionId,
        sequence: "1",
        stream: "stdout",
        text: "010_contract.hs written\n",
      });
    });
    expect(await screen.findByText(/010_contract\.hs written/)).toBeVisible();

    act(() => {
      publish({
        type: "finished",
        actionId,
        sequence: "2",
        status: "succeeded",
        exitCode: 0,
        finishedAtUnixMs: "1786853020000",
      });
    });
    await waitFor(() => expect(bridge.studio.refresh).toHaveBeenCalledOnce());
    expect(screen.getByRole("button", { name: "Close action output" })).toBeVisible();
  });

  it("runs namespace-qualified offline and live smoke actions from plugin cards", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("Workflow entries");
    await user.click(screen.getByRole("button", { name: "Plugins" }));

    const offlineButtons = await screen.findAllByRole("button", { name: /Offline smoke/ });
    const providerSmoke = offlineButtons.at(0);
    if (!providerSmoke) throw new Error("provider smoke button is missing");
    await user.click(providerSmoke);
    expect(bridge.actions.start).toHaveBeenLastCalledWith({
      kind: "smoke",
      targets: [{ namespace: "provider", name: "codex" }],
      live: false,
    });

    act(() => {
      publish({
        type: "finished",
        actionId,
        sequence: "1",
        status: "succeeded",
        exitCode: 0,
        finishedAtUnixMs: "1786853020000",
      });
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Close action output" })).toBeVisible(),
    );
    await user.click(screen.getByRole("button", { name: "Close action output" }));

    const liveButtons = screen.getAllByRole("button", { name: /Live smoke/ });
    const effectSmoke = liveButtons.at(1);
    if (!effectSmoke) throw new Error("effect smoke button is missing");
    await user.click(effectSmoke);
    expect(bridge.actions.start).toHaveBeenLastCalledWith({
      kind: "smoke",
      targets: [{ namespace: "effect", name: "workspace.paths" }],
      live: true,
    });
  });

  it("pages events for the selected run without reading trace files in the renderer", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("Workflow entries");
    await user.click(screen.getByRole("button", { name: "Runs" }));

    await waitFor(() =>
      expect(bridge.runs.events).toHaveBeenCalledWith({
        runId: "run-1786853000000-a1",
        after: "0",
        limit: 100,
      }),
    );
    expect(await screen.findByText("generation.started")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Load more events" }));
    await waitFor(() =>
      expect(bridge.runs.events).toHaveBeenLastCalledWith({
        runId: "run-1786853000000-a1",
        after: "1",
        limit: 100,
      }),
    );
    expect(await screen.findByText("provider.completed")).toBeVisible();
    expect(screen.getByRole("button", { name: "End of trace" })).toBeDisabled();
  });

  it("keeps one global action and exposes cancellation", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("Workflow entries");

    await user.click(screen.getByRole("button", { name: "Workflow" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(bridge.actions.start).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(bridge.actions.cancel).toHaveBeenCalledWith({ actionId });
  });
});

function fakeBridge(): {
  readonly bridge: MotivoBridge;
  readonly publish: (event: StudioActionEvent) => void;
} {
  let listener: ((event: StudioActionEvent) => void) | undefined;
  const view = studioView();
  const firstPage = eventPage(false);
  const secondPage: StudioEventPage = {
    ...firstPage,
    events: [
      {
        seq: "2",
        atUnixMs: "1786853000600",
        kind: "provider.completed",
        data: { provider: "codex", status: "ok" },
      },
    ],
    nextAfter: "2",
    complete: true,
    summary: {
      startedUnixMs: "1786853000000",
      finishedUnixMs: "1786853000900",
      eventsRecorded: "2",
      outcome: {
        kind: "succeeded",
        exitCode: 0,
        elapsedMs: "900",
        stderrTruncated: false,
      },
    },
  };
  const bridge: MotivoBridge = {
    studio: {
      current: vi.fn().mockResolvedValue(view),
      openInitialized: vi.fn().mockResolvedValue(view),
      initialize: vi.fn().mockResolvedValue(view),
      refresh: vi.fn().mockResolvedValue(view),
    },
    actions: {
      start: vi.fn().mockResolvedValue({
        actionId,
        kind: "generate",
        startedAtUnixMs: "1786853010000",
      }),
      cancel: vi.fn().mockResolvedValue(undefined),
      subscribe: vi.fn().mockImplementation((next: (event: StudioActionEvent) => void) => {
        listener = next;
        return vi.fn();
      }),
    },
    runs: {
      events: vi
        .fn()
        .mockResolvedValueOnce(firstPage)
        .mockResolvedValueOnce(secondPage)
        .mockResolvedValue(secondPage),
    },
  };
  return {
    bridge,
    publish: (event) => {
      if (!listener) throw new Error("action listener was not installed");
      listener(event);
    },
  };
}

function studioView(): StudioView {
  return {
    handle,
    snapshot: {
      api: "agenstro.studio/v1",
      generatedAtUnixMs: "1786853000000",
      workspace: { name: "topology-demo" },
      health: {
        ok: true,
        checks: [
          { name: "config", ok: true, detail: "Typed configuration loaded" },
          { name: "ghc", ok: true, detail: "ghc is available" },
        ],
      },
      scripts: [
        { relativePath: ".tactus/scripts/010_contract.hs", order: 10, runnable: true },
        { relativePath: ".tactus/scripts/020_count.hs", order: 20, runnable: true },
        { relativePath: ".tactus/scripts/Grid.hs", runnable: false },
      ],
      registries: {
        defaultProvider: "codex",
        providers: [
          {
            name: "codex",
            namespace: "provider",
            available: true,
            default: true,
            model: "gpt-5",
            effort: "high",
            observesInvocations: false,
          },
        ],
        effects: [
          {
            name: "workspace.paths",
            namespace: "effect",
            available: true,
            default: false,
            observesInvocations: true,
          },
        ],
        plugins: [],
      },
      runs: [
        {
          runId: "run-1786853000000-a1",
          state: "succeeded",
          integrity: "ok",
          startedUnixMs: "1786853000000",
          finishedUnixMs: "1786853000900",
          eventsRecorded: "2",
          label: "Generate with codex",
          namespace: "provider",
          subject: "codex",
          method: "generate",
          outcome: {
            kind: "succeeded",
            exitCode: 0,
            elapsedMs: "900",
            stderrTruncated: false,
          },
        },
      ],
    },
  };
}

function eventPage(complete: boolean): StudioEventPage {
  const run = studioView().snapshot.runs[0];
  if (!run) throw new Error("run fixture is missing");
  return {
    api: "agenstro.studio/v1",
    run,
    events: [
      {
        seq: "1",
        atUnixMs: "1786853000100",
        kind: "generation.started",
        data: { provider: "codex" },
      },
    ],
    nextAfter: "1",
    complete,
    integrity: "ok",
  };
}
