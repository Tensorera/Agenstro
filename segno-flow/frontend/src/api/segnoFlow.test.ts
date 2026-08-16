import { describe, expect, it } from "vitest";
import { segnoFlowApi } from "./segnoFlow";

describe("Segno Flow browser API", () => {
  it("returns normalized mock data", async () => {
    const [status, tasks] = await Promise.all([
      segnoFlowApi.systemStatus(),
      segnoFlowApi.taskList(),
    ]);

    expect(status.schedulerRunning).toBe(true);
    expect(status.installationRoot).toContain("AgentroTasks");
    expect(tasks).toHaveLength(3);
    expect(tasks[0]).toMatchObject({
      id: "research-digest",
      enabled: true,
      cron: "15 7 * * 1-5",
    });
  });

  it("keeps task updates durable across calls", async () => {
    const updated = await segnoFlowApi.taskSetEnabled("release-evidence", true);
    const tasks = await segnoFlowApi.taskList();

    expect(updated.enabled).toBe(true);
    expect(tasks.find((task) => task.id === updated.id)?.enabled).toBe(true);
  });

  it("surfaces import compilation details as a typed error", async () => {
    await expect(segnoFlowApi.taskImport("invalid.zip", "ZmFrZQ==")).rejects.toMatchObject(
      {
        name: "SegnoFlowApiError",
        errorType: "CompilationError",
        details: expect.arrayContaining([expect.stringContaining("cron is required")]),
      },
    );
  });

  it("normalizes the Python bridge task shape", async () => {
    window.pywebview = {
      api: {
        task_list: () => ({
          ok: true,
          data: {
            tasks: [{
              id: "bridge-task",
              name: "Bridge task",
              description: "Production-shaped fixture",
              cron: "0 4 * * *",
              timezone: "UTC",
              enabled: true,
              status: "timed_out",
              working_directory: "workspace",
              target_directory: "D:\\Tasks\\bridge-task\\workspace",
              task_directory: "D:\\Tasks\\bridge-task",
              scripts: {
                preprocess: "scripts/pre.py",
                main: "scripts/main.py",
                postprocess: "scripts/post.py",
                helpers: ["scripts/helpers.py"],
              },
              next_run_at: "2026-08-01T04:00:00Z",
              last_run: {
                run_id: "run-1",
                status: "timed_out",
                started_at: "2026-07-31T04:00:00Z",
                finished_at: "2026-07-31T04:30:00Z",
              },
              running: false,
            }],
          },
          error: null,
        }),
      },
    };

    const [task] = await segnoFlowApi.taskList();
    expect(task).toMatchObject({
      id: "bridge-task",
      status: "failed",
      lastRunAt: "2026-07-31T04:30:00Z",
      targetDirectory: "D:\\Tasks\\bridge-task\\workspace",
      scripts: {
        preprocess: "scripts/pre.py",
        main: "scripts/main.py",
        postprocess: "scripts/post.py",
      },
    });
  });

  it("preserves extended run and stage states from the Python bridge", async () => {
    window.pywebview = {
      api: {
        task_run_detail: () => ({
          ok: true,
          data: {
            run: {
              run_id: "run-2",
              task_id: "bridge-task",
              trigger: "schedule",
              status: "timed_out",
              created_at: "2026-07-31T05:00:00Z",
              started_at: null,
              finished_at: "2026-07-31T06:00:00Z",
              duration_seconds: 3600,
              error: "Task exceeded its timeout",
              phases: [
                { name: "pre", status: "succeeded", exit_code: 0 },
                { name: "main", status: "timed_out", exit_code: null },
                { name: "post", status: "skipped", exit_code: null },
              ],
              artifacts: [],
            },
            log: "preprocess complete\nmain process timed out",
          },
          error: null,
        }),
      },
    };

    const detail = await segnoFlowApi.taskRunDetail("bridge-task", "run-2");
    expect(detail.status).toBe("timed_out");
    expect(detail.startedAt).toBe("2026-07-31T05:00:00Z");
    expect(detail.durationMs).toBe(3_600_000);
    expect(detail.phases.map((phase) => [phase.name, phase.status])).toEqual([
      ["preprocess", "succeeded"],
      ["main", "timed_out"],
      ["postprocess", "skipped"],
    ]);
    expect(detail.logs).toHaveLength(2);
  });
});
