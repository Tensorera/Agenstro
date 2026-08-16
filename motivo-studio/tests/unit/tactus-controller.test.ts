import { Buffer } from "node:buffer";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { commandForAction, splitUtf8, TactusController } from "../../src/main/tactus/controller";
import type { StudioActionEvent } from "../../src/shared/contracts";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});

describe("Tactus command boundary", () => {
  it("builds fixed argv arrays without interpolating the goal or workspace", () => {
    const root = "D:\\work space\\demo";
    expect(
      commandForAction(
        { kind: "generate", goal: "say $(whoami) && keep spaces", provider: "codex" },
        root,
      ),
    ).toEqual([
      "generate",
      "--root",
      root,
      "--json",
      "--provider",
      "codex",
      "say $(whoami) && keep spaces",
    ]);
    expect(commandForAction({ kind: "check" }, root)).toEqual(["check", "--root", root]);
    expect(commandForAction({ kind: "run" }, root)).toEqual(["run", "--root", root]);
  });

  it("adds live smoke only as a fixed flag", () => {
    expect(
      commandForAction(
        {
          kind: "smoke",
          targets: [{ namespace: "provider", name: "codex" }],
          live: true,
        },
        "C:\\work",
      ),
    ).toEqual(["smoke", "--root", "C:\\work", "--json", "--live", "provider:codex"]);
  });

  it("splits projected output on Unicode scalar boundaries and byte limits", () => {
    const parts = splitUtf8("界".repeat(10), 10);
    expect(parts.join("")).toBe("界".repeat(10));
    expect(parts.every((part) => Buffer.byteLength(part, "utf8") <= 10)).toBe(true);
  });

  it("validates control JSON, redacts the root, and terminates output floods", async () => {
    const fixture = await fakeTactus(`
const args = process.argv.slice(2);
if (args[0] === "studio" && args[1] === "inspect") {
  const root = args[args.indexOf("--root") + 1];
  process.stdout.write(JSON.stringify({
    api: "tactus.control/v1",
    command: "studio.inspect",
    status: "completed",
    futureEnvelopeField: true,
    data: {
      api: "agenstro.studio/v1",
      generatedAtUnixMs: "1",
      workspace: { name: "fixture", futureWorkspaceField: true },
      health: { ok: true, checks: [{ name: "root", ok: true, detail: root }] },
      scripts: [],
      registries: { defaultProvider: "codex", providers: [], effects: [], plugins: [] },
      runs: [],
      futureSnapshotField: true
    }
  }));
} else if (args[0] === "studio" && args[1] === "events") {
  const root = args[args.indexOf("--root") + 1];
  const runId = args[2];
  const run = {
    runId,
    state: "succeeded",
    integrity: "ok",
    startedUnixMs: "1",
    finishedUnixMs: "2",
    eventsRecorded: "1",
    label: "fixture run",
    futureRunField: true,
    outcome: { kind: "succeeded", exitCode: 0, elapsedMs: "1", stderrTruncated: false }
  };
  process.stdout.write(JSON.stringify({
    api: "tactus.control/v1",
    command: "studio.events",
    status: "completed",
    data: {
      api: "agenstro.studio/v1",
      run,
      events: [{
        seq: "1",
        atUnixMs: "1",
        kind: "custom.future.event",
        data: { observedPath: root },
        futureEventField: true
      }],
      nextAfter: "1",
      complete: true,
      integrity: "ok"
    }
  }));
} else {
  process.stdout.write("x".repeat(1024));
  setTimeout(() => {}, 10000);
}
`);
    const events: StudioActionEvent[] = [];
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      actionOutputLimitBytes: 128,
      emit: (event) => events.push(event),
    });
    const view = await controller.open(fixture.root);
    expect(view.snapshot.health.checks[0]?.detail).toBe("<workspace>");
    expect(JSON.stringify(view)).not.toContain(fixture.root);
    const page = await controller.events("run-1", "0", 10);
    expect(page.events[0]?.kind).toBe("custom.future.event");
    expect(page.events[0]?.data).toEqual({ observedPath: "<workspace>" });
    expect(JSON.stringify(page)).not.toContain(fixture.root);

    const state = controller.start({ kind: "run" });
    const finished = await waitForFinished(events, state.actionId);
    expect(finished.status).toBe("failed");
    expect(finished.message).toContain("projection byte budget");
    controller.dispose();
  });

  it("rejects malformed UTF-8 on the control stdout channel", async () => {
    const fixture = await fakeTactus(`process.stdout.write(Buffer.from([0xff]));`);
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      emit: () => undefined,
    });
    await expect(controller.open(fixture.root)).rejects.toMatchObject({
      detail: { code: "invalid_control_utf8" },
    });
    controller.dispose();
  });
});

async function fakeTactus(body: string): Promise<{ root: string; script: string }> {
  const root = await mkdtemp(join(tmpdir(), "motivo-controller-"));
  temporaryDirectories.push(root);
  const script = join(root, "fake-tactus.mjs");
  await writeFile(script, body, "utf8");
  return { root, script };
}

async function waitForFinished(
  events: StudioActionEvent[],
  actionId: string,
): Promise<Extract<StudioActionEvent, { type: "finished" }>> {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    const event = events.find(
      (candidate): candidate is Extract<StudioActionEvent, { type: "finished" }> =>
        candidate.type === "finished" && candidate.actionId === actionId,
    );
    if (event) return event;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("timed out waiting for the fake Tactus action");
}
