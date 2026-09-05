import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  MotivoBridge,
  StudioActionEvent,
  StudioEventPage,
  StudioView,
} from "../shared/contracts";
import type { SessionList, SessionView } from "../shared/session-contracts";
import type { TaskDocument } from "../shared/task-contracts";
import App from "./App";

const actionId = "3ce53087-2218-42fd-bdda-afc4097020ae";
const handle = "aa665bbe-ece0-40e6-8235-2278635aee84";

interface Deferred<Value> {
  readonly promise: Promise<Value>;
  readonly resolve: (value: Value) => void;
}

function deferred<Value>(): Deferred<Value> {
  let resolve!: (value: Value) => void;
  const promise = new Promise<Value>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

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

  it("starts with a saved goal, then uses an explicit call budget and a boundary pause", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    expect(screen.getByRole("button", { name: "Tasks" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByLabelText("Workflow goal")).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Goal"), "Find and fix the import stall");
    await user.type(screen.getByLabelText(/Constraints and context/), "Preserve the file format.");
    await user.click(screen.getByRole("button", { name: /Save task/ }));
    expect(bridge.tasks.create).toHaveBeenCalledWith({
      workspaceHandle: handle,
      goal: "Find and fix the import stall",
      constraints: "Preserve the file format.",
      provider: "codex",
    });
    expect(
      await screen.findByRole("heading", { name: "Find and fix the import stall" }),
    ).toBeVisible();
    expect(bridge.tasks.continue).not.toHaveBeenCalled();
    expect(bridge.actions.start).not.toHaveBeenCalled();
    await user.selectOptions(screen.getByLabelText("Agent call budget for this attempt"), "2");
    await user.click(screen.getByRole("button", { name: "Begin task" }));
    expect(bridge.tasks.continue).toHaveBeenCalledWith({
      workspaceHandle: handle,
      taskId: taskDocument().id,
      maxCalls: 2,
    });
    expect(await screen.findByRole("button", { name: "Pause after current calls" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Workflow" }));
    expect(screen.getByRole("button", { name: "Select entries" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Check" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Tasks" }));
    await user.click(screen.getByRole("button", { name: "Pause after current calls" }));
    expect(bridge.tasks.pause).toHaveBeenCalledWith({
      workspaceHandle: handle,
      taskId: taskDocument().id,
    });
    expect(await screen.findByRole("button", { name: "Pause requested" })).toBeDisabled();
    expect(bridge.actions.cancel).not.toHaveBeenCalled();
  });

  it("shows sourced findings and reported checks, then returns the user's answer", async () => {
    const pending = reportedTask("needs_input");
    vi.mocked(bridge.tasks.list).mockResolvedValue([pending]);
    vi.mocked(bridge.tasks.current).mockResolvedValue(pending);
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByText("Can the import batch size be reduced?")).toBeVisible();
    expect(screen.getAllByRole("heading", { name: "Reported checks" })[0]).toBeVisible();
    expect(screen.getAllByText("Source: import.test.ts:42")[0]).toBeVisible();
    expect(screen.getAllByText("No source supplied · agent inference")[0]).toBeVisible();
    expect(screen.getAllByText(/Motivo has not independently verified/)[0]).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue task" })).toBeDisabled();
    await user.type(screen.getByLabelText("Your answer"), "Yes, use batches of 20.");
    await user.click(screen.getByRole("button", { name: "Continue task" }));
    expect(bridge.tasks.continue).toHaveBeenCalledWith({
      workspaceHandle: handle,
      taskId: pending.id,
      maxCalls: 4,
      note: "Yes, use batches of 20.",
    });
  });

  it("requires a checked external state and a substantive note before resuming an unknown outcome", async () => {
    const unknown = taskDocument({ status: "outcome_unknown", calls: 1 });
    vi.mocked(bridge.tasks.list).mockResolvedValue([unknown]);
    vi.mocked(bridge.tasks.current).mockResolvedValue(unknown);
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "Check what happened before continuing" });
    expect(bridge.tasks.continue).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Continue task" })).toBeDisabled();
    await user.type(
      screen.getByLabelText("What did you verify?"),
      "The import completed once; proceed to checking the output.",
    );
    expect(screen.getByRole("button", { name: "Continue task" })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: /I have checked the external state/ }));
    await user.click(screen.getByRole("button", { name: "Continue task" }));
    expect(bridge.tasks.continue).toHaveBeenCalledWith({
      workspaceHandle: handle,
      taskId: unknown.id,
      maxCalls: 4,
      note: "I checked the external outcome. The import completed once; proceed to checking the output.",
    });
  });

  it("reveals a failed response as plain text without interpreting its HTML", async () => {
    const rawOutput =
      '<img src="x" onerror="alert(1)"><script>alert(2)</script>\n[open](javascript:alert(3))';
    const failed = taskDocument({
      status: "failed",
      calls: 1,
      rounds: [
        {
          id: "1c64988b-580b-45f9-aa28-6f95eacaf222",
          role: "lead",
          focus: "Inspect malformed provider reply",
          startedAt: "2026-09-05T10:00:00.000Z",
          outcome: "failed",
          error: "The response could not be parsed.",
          rawOutput,
          rawOutputTruncated: true,
        },
      ],
    });
    vi.mocked(bridge.tasks.list).mockResolvedValue([failed]);
    vi.mocked(bridge.tasks.current).mockResolvedValue(failed);
    const user = userEvent.setup();
    render(<App />);
    const rawSummary = await screen.findByText("Raw agent response (truncated)");
    const rawPanel = rawSummary.closest("details");
    const response = rawPanel?.querySelector("pre");
    expect(response).not.toBeVisible();
    await user.click(screen.getByText("Inspect malformed provider reply"));
    expect(rawSummary).toBeVisible();
    expect(response).not.toBeVisible();
    await user.click(rawSummary);
    expect(response).toBeVisible();
    expect(response?.textContent).toBe(rawOutput);
    expect(rawPanel?.querySelector("img, script, a")).toBeNull();
    expect(bridge.tasks.continue).not.toHaveBeenCalled();
  });

  it("ignores a late task document after the person selects a different task", async () => {
    const oldCurrent = deferred<TaskDocument>();
    const first = taskDocument();
    const second = taskDocument({
      id: "0c0d2c44-36ba-47a8-9c37-c7ed251ce568",
      goal: "Investigate the export format",
    });
    vi.mocked(bridge.tasks.list).mockResolvedValue([first, second]);
    vi.mocked(bridge.tasks.current)
      .mockReturnValueOnce(oldCurrent.promise)
      .mockResolvedValue(second);
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: /Investigate the export format/ }));
    expect(await screen.findByRole("heading", { name: second.goal })).toBeVisible();
    await act(async () => oldCurrent.resolve(first));
    expect(screen.getByRole("heading", { name: second.goal })).toBeVisible();
    expect(screen.queryByRole("heading", { name: first.goal })).not.toBeInTheDocument();
  });

  it("ignores an old task query when the workspace changes", async () => {
    const oldCurrent = deferred<TaskDocument>();
    const first = taskDocument();
    const secondHandle = "bb665bbe-ece0-40e6-8235-2278635aee84";
    const nextStudio = { ...studioView(), handle: secondHandle };
    vi.mocked(bridge.tasks.list).mockResolvedValueOnce([first]).mockResolvedValue([]);
    vi.mocked(bridge.tasks.current).mockReturnValueOnce(oldCurrent.promise);
    vi.mocked(bridge.studio.openInitialized).mockResolvedValue(nextStudio);
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(bridge.tasks.current).toHaveBeenCalledOnce());
    await user.click(screen.getByRole("button", { name: "Open initialized workspace" }));
    await screen.findByText("What are you working toward?");
    await act(async () => oldCurrent.resolve(first));
    expect(screen.queryByRole("heading", { name: first.goal })).not.toBeInTheDocument();
    expect(bridge.tasks.list).toHaveBeenCalledWith({ workspaceHandle: secondHandle });
  });

  it("polls a working task to a saved pause without replaying it", async () => {
    const working = taskDocument({ status: "running", calls: 1 });
    const paused = reportedTask("paused");
    vi.mocked(bridge.tasks.list).mockResolvedValueOnce([working]).mockResolvedValue([paused]);
    vi.mocked(bridge.tasks.current).mockResolvedValueOnce(working).mockResolvedValue(paused);
    render(<App />);
    await screen.findByRole("button", { name: "Pause after current calls" });
    expect(
      await screen.findByRole("button", { name: "Continue task" }, { timeout: 3500 }),
    ).toBeEnabled();
    expect(bridge.tasks.current).toHaveBeenCalledTimes(2);
    expect(bridge.tasks.continue).not.toHaveBeenCalled();
  });

  it("checks selected helpers but only runs selected numbered entries", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Workflow" }));
    expect(screen.getByRole("button", { name: "Check" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: "Select Grid.hs" }));
    expect(screen.getByRole("button", { name: "Check" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: "Select 020_count.hs" }));
    await user.click(screen.getByRole("button", { name: "Run" }));
    expect(bridge.actions.start).toHaveBeenCalledWith({
      kind: "run",
      scripts: [".tactus/scripts/020_count.hs"],
    });
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
    expect(await screen.findByText("What are you working toward?")).toBeVisible();

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
        text: "[info] Wrote 010_contract.hs.\n",
        presentation: { category: "info", message: "Wrote 010_contract.hs." },
      });
    });
    expect(await screen.findByText("Wrote 010_contract.hs.")).toBeVisible();
    expect(screen.getByText("[info]")).toBeVisible();
    expect(screen.getByText("[info] Wrote 010_contract.hs.", { exact: false })).not.toBeVisible();

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

  it("reloads the Sessions page automatically after an action completes", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");

    await user.click(screen.getByRole("button", { name: "Sessions" }));
    await screen.findByText("A solid top exceeds the low-cost lift rating.");
    await user.click(screen.getByRole("button", { name: "Workflow" }));
    await user.click(screen.getByRole("checkbox", { name: "Select 010_contract.hs" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    await waitFor(() =>
      expect(bridge.actions.start).toHaveBeenCalledWith({
        kind: "check",
        scripts: [".tactus/scripts/010_contract.hs"],
      }),
    );
    await user.click(screen.getByRole("button", { name: "Sessions" }));

    vi.mocked(bridge.studio.refresh).mockClear();
    vi.mocked(bridge.sessions.list)
      .mockClear()
      .mockResolvedValue({
        api: "agenstro.session/v1",
        sessions: [deliveredSessionView()],
      });
    vi.mocked(bridge.sessions.current).mockClear().mockResolvedValue(deliveredSessionView());

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

    await waitFor(() => expect(bridge.studio.refresh).toHaveBeenCalledOnce());
    await waitFor(() =>
      expect(bridge.sessions.list).toHaveBeenCalledWith({ workspaceHandle: handle, limit: 50 }),
    );
    expect(await screen.findByText("Session delivered")).toBeVisible();
  });

  it("runs namespace-qualified offline and live smoke actions from plugin cards", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
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

  it("prefers canonical action messages and keeps raw failure diagnostics collapsed", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");

    act(() => {
      publish({
        type: "started",
        actionId,
        kind: "check",
        startedAtUnixMs: "1786853010000",
      });
      publish({
        type: "output",
        actionId,
        sequence: "1",
        stream: "stdout",
        text: "[state] Workflow check started.\n",
        presentation: { category: "state", message: "Workflow check started." },
      });
      publish({
        type: "output",
        actionId,
        sequence: "2",
        stream: "stdout",
        text: "[error] GHC rejected the selected source.\n",
        presentation: { category: "error", message: "GHC rejected the selected source." },
      });
      publish({
        type: "output",
        actionId,
        sequence: "3",
        stream: "stdout",
        text: "[info] Diagnostic capture finished.\n",
        presentation: { category: "info", message: "Diagnostic capture finished." },
      });
      publish({
        type: "finished",
        actionId,
        sequence: "4",
        status: "failed",
        exitCode: 1,
        finishedAtUnixMs: "1786853020000",
        message: '{"raw":"compiler diagnostic"}',
      });
    });

    expect(await screen.findByText("Workflow check started.")).toBeVisible();
    expect(screen.getAllByText("Workflow check started.")).toHaveLength(1);
    expect(screen.getByText("GHC rejected the selected source.")).toBeVisible();
    expect(screen.queryByText(/Type-check workflow failed/)).not.toBeInTheDocument();
    expect(screen.getByText(/compiler diagnostic/)).not.toBeVisible();

    await user.click(screen.getByText(/Technical details · stdout 3/));
    expect(screen.getByText(/compiler diagnostic/)).toBeVisible();
  });

  it("does not append a fallback terminal after a canonical state terminal", async () => {
    render(<App />);
    await screen.findByText("What are you working toward?");

    act(() => {
      publish({
        type: "started",
        actionId,
        kind: "check",
        startedAtUnixMs: "1786853010000",
      });
      publish({
        type: "output",
        actionId,
        sequence: "1",
        stream: "stderr",
        text: "[state] Workflow check started.\n",
        presentation: { category: "state", message: "Workflow check started." },
      });
      publish({
        type: "output",
        actionId,
        sequence: "2",
        stream: "stderr",
        text: "[state] Workflow check succeeded.\n",
        presentation: { category: "state", message: "Workflow check succeeded." },
      });
      publish({
        type: "output",
        actionId,
        sequence: "3",
        stream: "stderr",
        text: "[info] Run evidence was recorded.\n",
        presentation: { category: "info", message: "Run evidence was recorded." },
      });
      publish({
        type: "finished",
        actionId,
        sequence: "4",
        status: "succeeded",
        exitCode: 0,
        finishedAtUnixMs: "1786853020000",
      });
    });

    expect(await screen.findByText("Workflow check succeeded.")).toBeVisible();
    expect(screen.getByText("Run evidence was recorded.")).toBeVisible();
    expect(
      screen.queryByText("Type-check workflow completed successfully."),
    ).not.toBeInTheDocument();
  });

  it("pages events for the selected run without reading trace files in the renderer", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Runs" }));

    await waitFor(() =>
      expect(bridge.runs.events).toHaveBeenCalledWith({
        runId: "run-1786853000000-a1",
        after: "0",
        limit: 100,
      }),
    );
    expect(await screen.findByText("Generation started.")).toBeVisible();
    expect(screen.getByText("generation.started")).not.toBeVisible();
    await user.click(screen.getByText("Technical details · event #1"));
    expect(screen.getByText("generation.started")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Load more events" }));
    await waitFor(() =>
      expect(bridge.runs.events).toHaveBeenLastCalledWith({
        runId: "run-1786853000000-a1",
        after: "1",
        limit: 100,
      }),
    );
    expect(await screen.findByText("Provider completed.")).toBeVisible();
    expect(screen.getByText("provider.completed")).not.toBeVisible();
    expect(screen.getByRole("button", { name: "End of trace" })).toBeDisabled();
  });

  it("keeps legacy event payloads inside collapsed technical details", async () => {
    const user = userEvent.setup();
    const legacyPage = eventPage(true);
    vi.mocked(bridge.runs.events)
      .mockReset()
      .mockResolvedValue({
        ...legacyPage,
        events: [
          {
            seq: "9",
            atUnixMs: "1786853000100",
            kind: "legacy.raw_event",
            data: { diagnostic: "technical value" },
          },
        ],
        nextAfter: "9",
      });

    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Runs" }));

    const summary = await screen.findByText("Technical details · event #9");
    expect(screen.getByText("legacy.raw_event")).not.toBeVisible();
    expect(screen.getByText(/technical value/)).not.toBeVisible();
    await user.click(summary);
    expect(screen.getByText("legacy.raw_event")).toBeVisible();
    expect(screen.getByText(/technical value/)).toBeVisible();
  });

  it("keeps one global action and exposes cancellation", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");

    await user.click(screen.getByRole("button", { name: "Workflow" }));
    await user.click(screen.getByRole("checkbox", { name: "Select 010_contract.hs" }));
    await user.click(screen.getByRole("button", { name: "Check" }));
    expect(bridge.actions.start).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Run" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(bridge.actions.cancel).toHaveBeenCalledWith({ actionId });
  });

  it("renders a pending brief and returns one typed answer with its turn token", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");

    await user.click(screen.getByRole("button", { name: "Sessions" }));
    expect(await screen.findByText("A solid top exceeds the low-cost lift rating.")).toBeVisible();
    expect(bridge.sessions.list).toHaveBeenCalledWith({ workspaceHandle: handle, limit: 50 });
    expect(bridge.sessions.current).toHaveBeenCalledWith({
      workspaceHandle: handle,
      sessionId: "session-desk-1",
    });
    expect(screen.getByText("No source — inference")).toBeVisible();
    expect(screen.getAllByText("top-max-mm")).toHaveLength(2);
    expect(screen.getByText(/rules out solid oak/i)).toBeVisible();
    expect(screen.getAllByTitle("This consequence is irreversible.")).toHaveLength(2);
    expect(screen.getByText(/No default is available/)).toBeVisible();
    expect(screen.getByText("desk.cable-routing")).toBeVisible();

    await user.click(screen.getByRole("radio", { name: /Fixed height, built legs/ }));
    await user.type(screen.getByLabelText("Optional note"), "Prefer repairable joinery.");
    await user.click(screen.getByRole("button", { name: "Record answer" }));

    expect(bridge.sessions.answer).toHaveBeenCalledWith({
      workspaceHandle: handle,
      sessionId: "session-desk-1",
      turn: "3",
      axis: "desk.frame",
      option: "fixed",
      note: "Prefer repairable joinery.",
    });
    expect(await screen.findByText("Planning the next turn")).toBeVisible();
  });

  it.each([
    ["session_turn_stale", "The supplied turn is stale."],
    ["session_state_invalid", "The session state changed."],
  ])(
    "refetches recoverable %s answers without displaying the raw control error",
    async (code, message) => {
      const stale = new Error(message) as Error & {
        detail: { code: string };
      };
      stale.detail = { code };
      const refreshed = answeredSessionView();
      vi.mocked(bridge.sessions.answer).mockRejectedValueOnce(stale);
      vi.mocked(bridge.sessions.current)
        .mockResolvedValueOnce(pendingSessionView())
        .mockResolvedValue(refreshed);
      const user = userEvent.setup();
      render(<App />);
      await screen.findByText("What are you working toward?");
      await user.click(screen.getByRole("button", { name: "Sessions" }));
      await screen.findByText("A solid top exceeds the low-cost lift rating.");

      await user.click(screen.getByRole("radio", { name: /Fixed height, built legs/ }));
      await user.click(screen.getByRole("button", { name: "Record answer" }));

      expect(await screen.findByText("Planning the next turn")).toBeVisible();
      expect(screen.queryByText(message)).not.toBeInTheDocument();
      expect(bridge.sessions.current).toHaveBeenCalledTimes(2);
    },
  );

  it("keeps the decision disabled while the selected-session query is in flight", async () => {
    const current = deferred<SessionView>();
    vi.mocked(bridge.sessions.current).mockReset().mockReturnValue(current.promise);
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Sessions" }));

    await waitFor(() => expect(bridge.sessions.current).toHaveBeenCalledOnce());
    expect(await screen.findByRole("radio", { name: /Fixed height, built legs/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Refreshing/ })).toBeDisabled();

    await act(async () => current.resolve(pendingSessionView()));
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /Fixed height, built legs/ })).toBeEnabled(),
    );
  });

  it("shows a pending historical axis as a required revisit", async () => {
    const revisiting = pendingSessionView();
    revisiting.answered = [
      ...revisiting.answered,
      {
        axis: "desk.frame",
        option: "sit-stand",
        label: "Sit/stand, motorised frame",
        defaulted: false,
        answeredAtUnixMs: "1786852500000",
      },
    ];
    vi.mocked(bridge.sessions.list).mockResolvedValue({
      api: "agenstro.session/v1",
      sessions: [revisiting],
    });
    vi.mocked(bridge.sessions.current).mockResolvedValue(revisiting);
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Sessions" }));

    expect(await screen.findByText("revisiting")).toBeVisible();
    expect(screen.getByText("Must still decide (2)")).toBeVisible();
  });

  it("states the staged boundary without advertising a planner command", async () => {
    vi.mocked(bridge.sessions.list).mockResolvedValue({
      api: "agenstro.session/v1",
      sessions: [],
    });
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Sessions" }));

    expect(
      await screen.findByText(/No sessions are available in this staged boundary/),
    ).toBeVisible();
    expect(screen.getByText(/no planner or publish command/i)).toBeVisible();
    expect(screen.queryByText(/session producer/i)).not.toBeInTheDocument();
  });

  it("binds answers to a workspace token and ignores an older answer's finally", async () => {
    const opened = deferred<StudioView | null>();
    const oldAnswer = deferred<SessionView>();
    const newAnswer = deferred<SessionView>();
    const secondHandle = "bb665bbe-ece0-40e6-8235-2278635aee84";
    const secondView: StudioView = {
      ...studioView(),
      handle: secondHandle,
      snapshot: {
        ...studioView().snapshot,
        workspace: { name: "second-workspace" },
      },
    };
    vi.mocked(bridge.studio.openInitialized).mockReturnValue(opened.promise);
    vi.mocked(bridge.sessions.answer)
      .mockReset()
      .mockReturnValueOnce(oldAnswer.promise)
      .mockReturnValueOnce(newAnswer.promise);
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("What are you working toward?");
    await user.click(screen.getByRole("button", { name: "Sessions" }));
    await screen.findByText("A solid top exceeds the low-cost lift rating.");
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /Fixed height, built legs/ })).toBeEnabled(),
    );

    await user.click(screen.getByRole("button", { name: "Open initialized workspace" }));
    await user.click(screen.getByRole("radio", { name: /Fixed height, built legs/ }));
    await user.click(screen.getByRole("button", { name: "Record answer" }));
    expect(bridge.sessions.answer).toHaveBeenLastCalledWith(
      expect.objectContaining({ workspaceHandle: handle }),
    );

    await act(async () => opened.resolve(secondView));
    expect(await screen.findByText("second-workspace")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Sessions" }));
    await screen.findByText("A solid top exceeds the low-cost lift rating.");
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /Fixed height, built legs/ })).toBeEnabled(),
    );
    await user.click(screen.getByRole("radio", { name: /Fixed height, built legs/ }));
    await user.click(screen.getByRole("button", { name: "Record answer" }));
    expect(bridge.sessions.answer).toHaveBeenLastCalledWith(
      expect.objectContaining({ workspaceHandle: secondHandle }),
    );

    await user.click(screen.getByRole("button", { name: "Workflow" }));
    expect(screen.getByRole("button", { name: "Check" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Sessions" }));
    expect(screen.getByRole("button", { name: "Recording…" })).toBeDisabled();

    await act(async () => oldAnswer.resolve(answeredSessionView()));
    expect(screen.getByRole("button", { name: "Recording…" })).toBeDisabled();
    expect(screen.getByText("Recording…")).toBeVisible();

    await act(async () => newAnswer.resolve(answeredSessionView()));
    expect(await screen.findByText("Planning the next turn")).toBeVisible();
    expect(bridge.actions.start).not.toHaveBeenCalled();
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
        presentation: { category: "info", message: "Provider completed." },
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
  const pendingSession = pendingSessionView();
  const answeredSession = answeredSessionView();
  const sessionList: SessionList = {
    api: "agenstro.session/v1",
    sessions: [pendingSession, deliveredSessionView()],
  };
  let storedTask: TaskDocument | null = null;
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
    tasks: {
      list: vi.fn().mockImplementation(async () => (storedTask ? [storedTask] : [])),
      current: vi.fn().mockImplementation(async () => storedTask ?? taskDocument()),
      create: vi
        .fn()
        .mockImplementation(
          async (input: { goal: string; constraints: string; provider: string }) => {
            storedTask = { ...taskDocument(), ...input };
            return storedTask;
          },
        ),
      continue: vi.fn().mockImplementation(async () => {
        storedTask = {
          ...(storedTask ?? taskDocument()),
          status: "running",
          calls: 1,
          revision: 1,
        };
        return storedTask;
      }),
      pause: vi.fn().mockImplementation(async () => {
        storedTask = {
          ...(storedTask ?? taskDocument()),
          status: "running",
          pauseRequested: true,
          revision: 2,
        };
        return storedTask;
      }),
    },
    sessions: {
      list: vi.fn().mockResolvedValue(sessionList),
      current: vi.fn().mockResolvedValue(pendingSession),
      answer: vi.fn().mockResolvedValue(answeredSession),
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

function pendingSessionView(): SessionView {
  return {
    api: "agenstro.session/v1",
    sessionId: "session-desk-1",
    label: "Desk build",
    state: "awaiting_answer",
    turn: "3",
    pending: {
      api: "agenstro.session/v1",
      sessionId: "session-desk-1",
      turn: "3",
      findings: [
        {
          summary: "A solid top exceeds the low-cost lift rating.",
          detail: "The surveyed frames are rated for lighter tops.",
          source: "corpus: 40 commercial frames",
        },
        { summary: "The budget band was inferred from the selected frame." },
      ],
      question: {
        axis: "desk.frame",
        prompt: "Do you want the desk height to be adjustable?",
        reversibility: "irreversible",
        dependsOn: [],
        options: [
          {
            id: "sit-stand",
            label: "Sit/stand, motorised frame",
            coordinates: { height: "adjustable", cost: "high", "top-max-mm": "25" },
            rationale: "The frame carries the load.",
          },
          {
            id: "fixed",
            label: "Fixed height, built legs",
            coordinates: { height: "fixed", cost: "low", "top-max-mm": "45" },
            rationale: "The top can remain structural.",
          },
        ],
      },
      stakes: [
        {
          option: "sit-stand",
          effect: "Roughly doubles the budget and rules out solid oak.",
          reversibility: "irreversible",
        },
        {
          option: "fixed",
          effect: "Commits the height at assembly.",
          reversibility: "irreversible",
        },
      ],
      remainingSurface: ["desk.frame", "desk.top-material", "desk.joinery", "desk.cable-routing"],
      remainingFloor: ["desk.frame", "desk.top-material"],
    },
    answered: [
      {
        axis: "desk.budget",
        option: "mid",
        label: "Mid-range",
        defaulted: false,
        answeredAtUnixMs: "1786852000000",
      },
    ],
    startedUnixMs: "1786851000000",
    updatedUnixMs: "1786853000000",
  };
}

function answeredSessionView(): SessionView {
  const { pending: _pending, ...current } = pendingSessionView();
  void _pending;
  return {
    ...current,
    state: "planning",
    answered: [
      ...current.answered,
      {
        axis: "desk.frame",
        option: "fixed",
        label: "Fixed height, built legs",
        defaulted: false,
        answeredAtUnixMs: "1786853100000",
      },
    ],
    updatedUnixMs: "1786853100000",
  };
}

function deliveredSessionView(): SessionView {
  return {
    ...answeredSessionView(),
    sessionId: "session-desk-2",
    label: "Reading desk",
    state: "delivered",
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
        presentation: { category: "state", message: "Generation started." },
      },
    ],
    nextAfter: "1",
    complete,
    integrity: "ok",
  };
}

function taskDocument(overrides: Partial<TaskDocument> = {}): TaskDocument {
  return {
    api: "motivo.task/v1",
    id: "ed32292f-bf89-42dc-88ed-e4d95687b832",
    goal: "Make importing large documents reliable",
    constraints: "Keep existing documents readable.",
    provider: "codex",
    status: "ready",
    createdAt: "2026-09-05T10:00:00.000Z",
    updatedAt: "2026-09-05T10:00:00.000Z",
    revision: 0,
    calls: 0,
    pauseRequested: false,
    notes: [],
    rounds: [],
    ...overrides,
  };
}

function reportedTask(status: TaskDocument["status"]): TaskDocument {
  return taskDocument({
    status,
    calls: 1,
    revision: 2,
    rounds: [
      {
        id: "1c64988b-580b-45f9-aa28-6f95eacaf222",
        role: "lead",
        focus: "Find where the import waits",
        startedAt: "2026-09-05T10:00:00.000Z",
        finishedAt: "2026-09-05T10:00:03.000Z",
        elapsedMs: 3000,
        outcome: "succeeded",
        report: {
          action: "investigate",
          focus: "Find where the import waits",
          summary: "The importer holds a batch until every document is decoded.",
          findings: [
            {
              statement: "The batch test times out at 100 documents.",
              source: "import.test.ts:42",
            },
            { statement: "Smaller batches may avoid the peak memory pressure." },
          ],
          unknowns: ["Whether lowering the batch size changes throughput."],
          decision: "Measure smaller batches before changing the parser.",
          artifacts: ["notes/import-observations.md"],
          checks: [
            {
              name: "Small import test",
              result: "passed",
              detail: "The test passed for 10 documents.",
            },
          ],
          next: "Compare throughput with batches of 20.",
          status: status === "needs_input" ? "needs_input" : "continue",
          ...(status === "needs_input"
            ? { question: "Can the import batch size be reduced?" }
            : {}),
        },
      },
    ],
  });
}
