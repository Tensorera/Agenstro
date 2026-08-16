import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "./App";
import { segnoFlowApi } from "./api/segnoFlow";

describe("Segno Flow workspace", () => {
  it("restores tasks, run history, and log detail from the browser mock", async () => {
    render(<App />);

    expect(await screen.findByRole("heading", { level: 1, name: "Research digest" })).toBeVisible();
    expect(screen.getByText("Scheduler online")).toBeVisible();
    expect(screen.getAllByText("15 7 * * 1-5")).toHaveLength(2);
    expect(await screen.findByText("Created isolated work directory and collected 14 inputs")).toBeVisible();
    expect(screen.getByRole("button", { name: /Run now/i })).toBeEnabled();
  });

  it("navigates between tasks and exposes an active run", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { level: 1, name: "Research digest" });

    await user.click(screen.getByRole("button", { name: /Feedback sweep/i }));

    expect(await screen.findByRole("heading", { level: 1, name: "Feedback sweep" })).toBeVisible();
    expect(await screen.findByText("Workflow is collecting source material")).toBeVisible();
    expect(screen.getAllByRole("button", { name: /^Running$/i }).some((button) => button.hasAttribute("disabled"))).toBe(true);
  });

  it("changes whether a schedule is enabled", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { level: 1, name: "Research digest" });

    await user.click(screen.getByRole("button", { name: /Release evidence pack/i }));
    const pauseToggle = await screen.findByRole("button", { name: "Enable schedule" });
    expect(pauseToggle).toHaveAttribute("aria-pressed", "false");
    await user.click(pauseToggle);

    expect(await screen.findByRole("button", { name: "Pause schedule" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("status")).toHaveTextContent("Release evidence pack is now enabled");
  });

  it("starts a manual run and selects it in history", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { level: 1, name: "Research digest" });

    await user.click(screen.getByRole("button", { name: /Run now/i }));

    expect(await screen.findByRole("status")).toHaveTextContent(/was accepted/i);
    await waitFor(() => {
      expect(screen.getByText("Manual")).toBeVisible();
    });
  });

  it("polls the selected task and refreshes an active run without stealing selection", async () => {
    const user = userEvent.setup();
    const originalTaskRuns = segnoFlowApi.taskRuns;
    const originalRunDetail = segnoFlowApi.taskRunDetail;
    let prependNewRun = false;
    let refreshedDetail = false;
    const taskRunsSpy = vi.spyOn(segnoFlowApi, "taskRuns").mockImplementation(async (taskId) => {
      const current = await originalTaskRuns(taskId);
      if (taskId !== "feedback-sweep" || !prependNewRun) return current;
      return [{
        id: "feedback-sweep-queued-new",
        taskId,
        status: "queued",
        trigger: "schedule",
        startedAt: "2026-08-01T06:00:00Z",
        finishedAt: null,
        durationMs: null,
        summary: "New scheduled run",
      }, ...current];
    });
    const detailSpy = vi.spyOn(segnoFlowApi, "taskRunDetail").mockImplementation(async (taskId, runId) => {
      const detail = await originalRunDetail(taskId, runId);
      return refreshedDetail && runId === "feedback-sweep-218"
        ? {
            ...detail,
            logs: [...detail.logs, {
              timestamp: "2026-08-01T06:01:00Z",
              phase: "system",
              level: "info",
              message: "Active detail refreshed by polling",
            }],
          }
        : detail;
    });
    const intervals: Array<{ handler: TimerHandler; delay?: number }> = [];
    vi.spyOn(window, "setInterval").mockImplementation((handler, delay) => {
      intervals.push({ handler, delay });
      return intervals.length;
    });
    vi.spyOn(window, "clearInterval").mockImplementation(() => undefined);

    render(<App />);
    await screen.findByRole("heading", { level: 1, name: "Research digest" });
    await user.click(screen.getByRole("button", { name: /Feedback sweep/i }));
    await screen.findByRole("heading", { level: 1, name: "Feedback sweep" });
    await screen.findByText("Workflow is collecting source material");
    const history = screen.getByRole("region", { name: "Run history" });
    expect(history.querySelector('button[aria-current="true"]')).toHaveTextContent(
      "Workflow is collecting source material",
    );

    const detailCallsBeforePoll = detailSpy.mock.calls.length;
    const historyCallsBeforePoll = taskRunsSpy.mock.calls.length;
    prependNewRun = true;
    refreshedDetail = true;
    const runPolls = intervals.filter((interval) => interval.delay === 2_500);
    expect(runPolls.length).toBeGreaterThan(0);
    const selectedTaskPoll = runPolls.at(-1);
    expect(typeof selectedTaskPoll?.handler).toBe("function");
    await act(async () => {
      if (typeof selectedTaskPoll?.handler === "function") selectedTaskPoll.handler();
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(taskRunsSpy.mock.calls.length).toBeGreaterThan(historyCallsBeforePoll));
    await waitFor(() => expect(detailSpy.mock.calls.length).toBeGreaterThan(detailCallsBeforePoll));
    expect(await screen.findByText("Active detail refreshed by polling")).toBeVisible();
    expect(history.querySelectorAll("button")).toHaveLength(2);
    expect(history.querySelector('button[aria-current="true"]')).toHaveTextContent(
      "Workflow is collecting source material",
    );
    expect(history.querySelector('button[aria-current="true"]')).not.toHaveTextContent(
      "New scheduled run",
    );
  });

  it("imports and selects a ZIP package", async () => {
    const user = userEvent.setup();
    const { container } = render(<App />);
    await screen.findByRole("heading", { level: 1, name: "Research digest" });

    await user.click(screen.getByRole("button", { name: "Import task package" }));
    const dialog = screen.getByRole("dialog", { name: "Import task package" });
    const input = container.querySelector<HTMLInputElement>('input[type="file"]');
    expect(input).not.toBeNull();
    await user.upload(
      input!,
      new File(["mock task package"], "daily-report.zip", { type: "application/zip" }),
    );
    expect(within(dialog).getByText("daily-report.zip")).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Validate & import" }));

    expect(await screen.findByRole("heading", { level: 1, name: "Daily Report" })).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent(/imported with 1 warning/i);
  });

  it("shows compiler diagnostics without closing the import dialog", async () => {
    const user = userEvent.setup();
    const { container } = render(<App />);
    await screen.findByRole("heading", { level: 1, name: "Research digest" });

    await user.click(screen.getByRole("button", { name: "Import task package" }));
    const input = container.querySelector<HTMLInputElement>('input[type="file"]');
    await user.upload(
      input!,
      new File(["invalid"], "invalid-workflow.zip", { type: "application/zip" }),
    );
    await user.click(screen.getByRole("button", { name: "Validate & import" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("did not pass compilation checks");
    expect(alert).toHaveTextContent("cron is required");
    expect(screen.getByRole("dialog", { name: "Import task package" })).toBeVisible();
  });

  it("provides an actionable empty state", async () => {
    window.pywebview = {
      api: {
        system_status: () => ({
          ok: true,
          data: {
            root: "C:\\Tasks",
            service: { running: true, started_at: "2026-07-31T12:00:00Z" },
            scheduler: { running: true },
            task_count: 0,
            enabled_count: 0,
            running_count: 0,
          },
          error: null,
        }),
        task_list: () => ({ ok: true, data: { tasks: [] }, error: null }),
      },
    };
    render(<App />);

    expect(await screen.findByRole("heading", { level: 1, name: /Put recurring work/i })).toBeVisible();
    expect(screen.getByRole("button", { name: "Import your first task" })).toBeEnabled();
  });
});
