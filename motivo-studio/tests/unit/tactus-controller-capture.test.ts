import { EventEmitter } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import { afterEach, describe, expect, it, vi } from "vitest";

const { spawnMock } = vi.hoisted(() => ({ spawnMock: vi.fn() }));

vi.mock("node:child_process", () => ({ default: { spawn: spawnMock }, spawn: spawnMock }));

import { TactusController } from "../../src/main/tactus/controller";

class FakeChild extends EventEmitter {
  readonly pid = 41;
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  readonly kill = vi.fn(() => true);
}

const temporaryDirectories: string[] = [];

afterEach(async () => {
  vi.useRealTimers();
  spawnMock.mockReset();
  await Promise.all(
    temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});

describe("Tactus control child lifetime", () => {
  it("keeps the control lock after kill grace until the child closes", async () => {
    const root = await mkdtemp(join(tmpdir(), "motivo-capture-"));
    temporaryDirectories.push(root);
    const children: FakeChild[] = [];
    spawnMock.mockImplementation(() => {
      const child = new FakeChild();
      children.push(child);
      return child;
    });
    const controller = new TactusController({ emit: vi.fn() });

    const opening = controller.open(root);
    await waitForChild(children, 1);
    children[0]?.stdout.write(
      JSON.stringify({
        api: "tactus.control/v1",
        command: "studio.inspect",
        status: "completed",
        data: {
          api: "agenstro.studio/v1",
          generatedAtUnixMs: "1",
          workspace: { name: "fixture" },
          health: { ok: true, checks: [] },
          scripts: [
            { relativePath: ".tactus/scripts/010_main.hs", order: 10, runnable: true },
            { relativePath: ".tactus/scripts/Support.hs", runnable: false },
          ],
          registries: { defaultProvider: "codex", providers: [], effects: [], plugins: [] },
          runs: [],
        },
      }),
    );
    children[0]?.emit("close", 0);
    const studio = await opening;

    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const listing = controller.sessions({ workspaceHandle: studio.handle, limit: 50 });
    const rejection = expect(listing).rejects.toMatchObject({
      detail: { code: "control_output_too_large" },
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(children).toHaveLength(2);
    children[1]?.stdout.emit("data", Buffer.alloc(9 * 1_024 * 1_024 + 1));
    await vi.advanceTimersByTimeAsync(2_000);
    await rejection;

    expect(children[1]?.kill).toHaveBeenCalledWith("SIGKILL");
    const queuedListing = controller.sessions({ workspaceHandle: studio.handle, limit: 50 });
    await Promise.resolve();
    expect(children).toHaveLength(2);
    expect(() =>
      controller.start({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] }),
    ).toThrowError(
      expect.objectContaining({ detail: expect.objectContaining({ code: "control_busy" }) }),
    );

    children[1]?.emit("close", null);
    vi.useRealTimers();
    await waitForChild(children, 3);
    children[2]?.stdout.write(
      JSON.stringify({
        api: "tactus.control/v1",
        command: "session.list",
        status: "completed",
        data: { api: "agenstro.session/v1", sessions: [] },
      }),
    );
    children[2]?.emit("close", 0);
    await expect(queuedListing).resolves.toMatchObject({ sessions: [] });
    expect(() =>
      controller.start({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] }),
    ).not.toThrow();
    controller.dispose();
  });
});

async function waitForChild(children: readonly FakeChild[], count: number): Promise<void> {
  while (children.length < count) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}
