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
          scripts: [],
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
    children[1]?.stdout.emit("data", Buffer.alloc(9 * 1_024 * 1_024 + 1));
    await vi.advanceTimersByTimeAsync(2_000);
    await rejection;

    expect(children[1]?.kill).toHaveBeenCalledWith("SIGKILL");
    expect(() => controller.start({ kind: "run" })).toThrowError(
      expect.objectContaining({ detail: expect.objectContaining({ code: "control_busy" }) }),
    );
    await expect(
      controller.sessions({ workspaceHandle: studio.handle, limit: 50 }),
    ).rejects.toMatchObject({ detail: { code: "control_busy" } });

    children[1]?.emit("close", null);
    expect(() => controller.start({ kind: "run" })).not.toThrow();
    controller.dispose();
  });
});

async function waitForChild(children: readonly FakeChild[], count: number): Promise<void> {
  while (children.length < count) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}
