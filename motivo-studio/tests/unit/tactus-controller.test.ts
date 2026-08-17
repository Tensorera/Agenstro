import { Buffer } from "node:buffer";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  commandForAction,
  parseTactusPresentation,
  splitUtf8,
  TactusController,
} from "../../src/main/tactus/controller";
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
    ).toEqual(["generate", "--root", root, "--provider", "codex", "say $(whoami) && keep spaces"]);
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
    ).toEqual(["smoke", "--root", "C:\\work", "--live", "provider:codex"]);
  });

  it("splits projected output on Unicode scalar boundaries and byte limits", () => {
    const parts = splitUtf8("界".repeat(10), 10);
    expect(parts.join("")).toBe("界".repeat(10));
    expect(parts.every((part) => Buffer.byteLength(part, "utf8") <= 10)).toBe(true);
  });

  it("projects only exact canonical human presentation lines", () => {
    expect(parseTactusPresentation("[state] Workflow started.\r\n")).toEqual({
      category: "state",
      message: "Workflow started.",
    });
    expect(parseTactusPresentation("[warning] Outcome is ambiguous.\n")).toEqual({
      category: "warning",
      message: "Outcome is ambiguous.",
    });
    for (const raw of [
      "progress on stderr\n",
      "prefix [error] not canonical\n",
      "[INFO] wrong case\n",
      "[error]\n",
      "[debug] unsupported\n",
    ]) {
      expect(parseTactusPresentation(raw)).toBeUndefined();
    }
  });

  it("validates control JSON, redacts the root, and degrades output floods without killing Tactus", async () => {
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
        presentation: { category: "warning", message: "Future evidence is incomplete." },
        data: { observedPath: root },
        futureEventField: true
      }],
      nextAfter: "1",
      complete: true,
      integrity: "ok"
    }
  }));
} else if (args[0] === "check") {
  process.stdout.write("[state] Workflow");
  setTimeout(() => {
    process.stdout.write(" check started.\\nraw technical line\\n");
    process.stderr.write("normal progress on stderr\\n");
  }, 5);
} else {
  process.stdout.write("x".repeat(256 * 1024));
  process.stderr.write("y".repeat(256 * 1024));
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
    expect(page.events[0]?.presentation).toEqual({
      category: "warning",
      message: "Future evidence is incomplete.",
    });
    expect(page.events[0]?.data).toEqual({ observedPath: "<workspace>" });
    expect(JSON.stringify(page)).not.toContain(fixture.root);

    const state = controller.start({ kind: "run" });
    const finished = await waitForFinished(events, state.actionId);
    expect(finished.status).toBe("succeeded");
    expect(finished.exitCode).toBe(0);
    expect(finished.message).toBeUndefined();
    const floodWarnings = actionOutput(events, state.actionId).filter(
      (event) => event.presentation?.category === "warning",
    );
    expect(floodWarnings).toHaveLength(1);
    expect(floodWarnings[0]?.presentation?.message).toContain("output was omitted");

    const checked = controller.start({ kind: "check" });
    await waitForFinished(events, checked.actionId);
    const checkOutput = events.filter(
      (event): event is Extract<StudioActionEvent, { type: "output" }> =>
        event.type === "output" && event.actionId === checked.actionId,
    );
    expect(checkOutput.find((event) => event.presentation)?.presentation).toEqual({
      category: "state",
      message: "Workflow check started.",
    });
    expect(
      checkOutput.find((event) => event.text.includes("normal progress on stderr"))?.presentation,
    ).toBeUndefined();
    controller.dispose();
  });

  it("keeps the child outcome when the projected frame budget is exhausted", async () => {
    const fixture = await fakeActionTactus(`
if (args[0] !== "run") process.exitCode = 2;
process.stdout.write("[info] First projected line.\\n");
process.stdout.write("[info] Second projected line.\\n");
process.stdout.write("[info] Third projected line.\\n");
`);
    const events: StudioActionEvent[] = [];
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      actionOutputLimitFrames: 1,
      emit: (event) => events.push(event),
    });
    await controller.open(fixture.root);

    const state = controller.start({ kind: "run" });
    const finished = await waitForFinished(events, state.actionId);
    expect(finished.status).toBe("succeeded");
    expect(finished.exitCode).toBe(0);
    const output = actionOutput(events, state.actionId);
    expect(output.filter((event) => event.presentation?.category === "warning")).toHaveLength(1);
    expect(output.map((event) => event.presentation?.category)).toEqual(["info", "warning"]);
    controller.dispose();
  });

  it("runs generate and smoke in human mode and projects their canonical diagnostics", async () => {
    const fixture = await fakeActionTactus(`
if (args.includes("--json")) {
  process.stderr.write("machine mode was not expected\\n");
  process.exitCode = 9;
} else if (args[0] === "generate") {
  process.stderr.write("[state] Workflow generation started.\\n");
  process.stderr.write("[error] Provider request failed.\\n");
  process.exitCode = 1;
} else if (args[0] === "smoke") {
  process.stderr.write("[state] Plugin smoke test started.\\n");
  process.stderr.write("[warning] The provider outcome is unknown; Tactus did not retry it.\\n");
} else {
  process.exitCode = 2;
}
`);
    const events: StudioActionEvent[] = [];
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      emit: (event) => events.push(event),
    });
    await controller.open(fixture.root);

    const generated = controller.start({
      kind: "generate",
      goal: "keep this as one argument",
      provider: "codex",
    });
    expect((await waitForFinished(events, generated.actionId)).status).toBe("failed");
    expect(actionOutput(events, generated.actionId).map((event) => event.presentation)).toEqual([
      { category: "state", message: "Workflow generation started." },
      { category: "error", message: "Provider request failed." },
    ]);

    const smoked = controller.start({
      kind: "smoke",
      targets: [{ namespace: "provider", name: "codex" }],
      live: true,
    });
    expect((await waitForFinished(events, smoked.actionId)).status).toBe("succeeded");
    expect(actionOutput(events, smoked.actionId).map((event) => event.presentation)).toEqual([
      { category: "state", message: "Plugin smoke test started." },
      {
        category: "warning",
        message: "The provider outcome is unknown; Tactus did not retry it.",
      },
    ]);
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

async function fakeActionTactus(actionBody: string): Promise<{ root: string; script: string }> {
  return fakeTactus(`
const args = process.argv.slice(2);
if (args[0] === "studio" && args[1] === "inspect") {
  process.stdout.write(JSON.stringify({
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
      runs: []
    }
  }));
} else {
${actionBody}
}
`);
}

function actionOutput(
  events: readonly StudioActionEvent[],
  actionId: string,
): Array<Extract<StudioActionEvent, { type: "output" }>> {
  return events.filter(
    (event): event is Extract<StudioActionEvent, { type: "output" }> =>
      event.type === "output" && event.actionId === actionId,
  );
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
