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
    expect(
      commandForAction({ kind: "check", scripts: [".tactus/scripts/010_main.hs"] }, root),
    ).toEqual(["check", "--root", root, ".tactus/scripts/010_main.hs"]);
    expect(
      commandForAction({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] }, root),
    ).toEqual(["run", "--root", root, "--script", ".tactus/scripts/010_main.hs"]);
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

  it("rejects paths absent from the snapshot and helpers selected for execution", async () => {
    const fixture = await fakeActionTactus('throw new Error("invalid selection dispatched");');
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      emit: () => undefined,
    });
    await controller.open(fixture.root);
    for (const scripts of [
      ["/other/010_main.hs"],
      ["../010_main.hs"],
      [".tactus/scripts/Support.hs"],
    ]) {
      expect(() => controller.start({ kind: "run", scripts })).toThrowError(
        expect.objectContaining({
          detail: expect.objectContaining({ code: "script_selection_invalid" }),
        }),
      );
    }
    controller.dispose();
  });

  it("binds every task operation to the selected workspace and validates provider availability", async () => {
    const fixture = await fakeActionTactus('throw new Error("unexpected provider invocation");');
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      emit: () => undefined,
    });
    const view = await controller.open(fixture.root);
    const stale = "bb665bbe-ece0-40e6-8235-2278635aee84";
    const taskId = "cc665bbe-ece0-40e6-8235-2278635aee84";
    for (const request of [
      () => controller.taskList({ workspaceHandle: stale }),
      () => controller.taskCurrent({ workspaceHandle: stale, taskId }),
      () =>
        controller.taskCreate({
          workspaceHandle: stale,
          goal: "fix parser",
          constraints: "",
          provider: "codex",
        }),
      () => controller.taskContinue({ workspaceHandle: stale, taskId, maxCalls: 1 }),
      () => controller.taskPause({ workspaceHandle: stale, taskId }),
    ]) {
      expect(request).toThrowError(
        expect.objectContaining({
          detail: expect.objectContaining({ code: "workspace_handle_stale" }),
        }),
      );
    }
    expect(() =>
      controller.taskCreate({
        workspaceHandle: view.handle,
        goal: "fix parser",
        constraints: "",
        provider: "codex",
      }),
    ).toThrowError(
      expect.objectContaining({
        detail: expect.objectContaining({ code: "provider_unavailable" }),
      }),
    );
    expect(await controller.taskList({ workspaceHandle: view.handle })).toEqual([]);
    controller.dispose();
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
      scripts: [{ relativePath: ".tactus/scripts/010_main.hs", order: 10, runnable: true }, { relativePath: ".tactus/scripts/Support.hs", runnable: false }],
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

    const state = controller.start({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] });
    const finished = await waitForFinished(events, state.actionId);
    expect(finished.status).toBe("succeeded");
    expect(finished.exitCode).toBe(0);
    expect(finished.message).toBeUndefined();
    const floodWarnings = actionOutput(events, state.actionId).filter(
      (event) => event.presentation?.category === "warning",
    );
    expect(floodWarnings).toHaveLength(1);
    expect(floodWarnings[0]?.presentation?.message).toContain("output was omitted");

    const checked = controller.start({ kind: "check", scripts: [".tactus/scripts/010_main.hs"] });
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

    const state = controller.start({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] });
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

  it("serializes overlapping control queries without allowing an action to overtake them", async () => {
    const fixture = await fakeTactus(`
const args = process.argv.slice(2);
const success = (command, data) => ({
  api: "tactus.control/v1",
  command,
  status: "completed",
  data
});
if (args[0] === "studio" && args[1] === "inspect") {
  process.stdout.write(JSON.stringify(success("studio.inspect", {
    api: "agenstro.studio/v1",
    generatedAtUnixMs: "1",
    workspace: { name: "fixture" },
    health: { ok: true, checks: [] },
    scripts: [{ relativePath: ".tactus/scripts/010_main.hs", order: 10, runnable: true }, { relativePath: ".tactus/scripts/Support.hs", runnable: false }],
    registries: { defaultProvider: "codex", providers: [], effects: [], plugins: [] },
    runs: []
  })));
} else if (args[0] === "session" && args[1] === "list") {
  setTimeout(() => process.stdout.write(JSON.stringify(success("session.list", {
    api: "agenstro.session/v1",
    sessions: []
  }))), 75);
} else if (args[0] === "run") {
  setTimeout(() => process.stdout.write("[state] run complete\\n"), 75);
} else {
  process.exitCode = 9;
}
`);
    const events: StudioActionEvent[] = [];
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      emit: (event) => events.push(event),
    });
    const view = await controller.open(fixture.root);

    const pendingRefresh = controller.refresh();
    const pendingList = controller.sessions({ workspaceHandle: view.handle, limit: 25 });
    let controlBusy: unknown;
    try {
      controller.start({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] });
    } catch (error) {
      controlBusy = error;
    }
    expect(controlBusy).toMatchObject({ detail: { code: "control_busy", category: "busy" } });
    await expect(Promise.all([pendingRefresh, pendingList])).resolves.toMatchObject([
      { handle: view.handle },
      { sessions: [] },
    ]);

    const action = controller.start({ kind: "run", scripts: [".tactus/scripts/010_main.hs"] });
    await expect(
      controller.sessions({ workspaceHandle: view.handle, limit: 25 }),
    ).rejects.toMatchObject({ detail: { code: "action_busy", category: "busy" } });
    await waitForFinished(events, action.actionId);
    controller.dispose();
  });

  it("projects additive session controls, builds fixed answer argv, and classifies failures", async () => {
    const fixture = await fakeTactus(`
const args = process.argv.slice(2);
const root = args[args.indexOf("--root") + 1];
const pending = {
  api: "agenstro.session/v1",
  sessionId: "session-desk-1",
  label: "Desk build",
  state: "awaiting_answer",
  turn: "3",
  pending: {
    api: "agenstro.session/v1",
    sessionId: "session-desk-1",
    turn: "3",
    findings: [{ summary: "Grounded finding", source: root, futureFindingField: true }],
    question: {
      axis: "desk.frame",
      prompt: "Choose a frame.",
      reversibility: "irreversible",
      dependsOn: [],
      options: [
        { id: "fixed", label: "Fixed", coordinates: { [root]: "fixed" }, futureOptionField: true },
        { id: "moving", label: "Moving", coordinates: { height: "adjustable" } }
      ],
      futureQuestionField: true
    },
    stakes: [{
      option: "fixed",
      effect: "Commits the height.",
      reversibility: "irreversible",
      futureStakeField: true
    }],
    remainingSurface: ["desk.frame", "desk.finish"],
    remainingFloor: ["desk.frame"],
    futureBriefField: true
  },
  answered: [],
  startedUnixMs: "1",
  updatedUnixMs: "2",
  futureSessionField: true
};
const success = (command, data) => ({
  api: "tactus.control/v1",
  command,
  status: "completed",
  data,
  futureEnvelopeField: true
});
const failure = (command, code) => ({
  api: "tactus.control/v1",
  command,
  status: "error",
  error: { code, message: "Session control failed at " + root, futureErrorField: true }
});
if (args[0] === "studio" && args[1] === "inspect") {
  if (!args.includes("--exact-root")) process.exitCode = 8;
  process.stdout.write(JSON.stringify(success("studio.inspect", {
    api: "agenstro.studio/v1",
    generatedAtUnixMs: "1",
    workspace: { name: "fixture" },
    health: { ok: true, checks: [] },
    scripts: [{ relativePath: ".tactus/scripts/010_main.hs", order: 10, runnable: true }, { relativePath: ".tactus/scripts/Support.hs", runnable: false }],
    registries: { defaultProvider: "codex", providers: [], effects: [], plugins: [] },
    runs: []
  })));
} else if (args[0] === "session" && args[1] === "list") {
  if (args[args.indexOf("--limit") + 1] !== "25") process.exitCode = 8;
  process.stdout.write(JSON.stringify(success("session.list", {
    api: "agenstro.session/v1",
    sessions: [pending],
    futureListField: true
  })));
} else if (args[0] === "session" && args[1] === "show") {
  const requested = args[args.indexOf("--session") + 1];
  if (requested === "session-collision-1") {
    const collision = JSON.parse(JSON.stringify(pending));
    collision.sessionId = requested;
    collision.pending.sessionId = requested;
    collision.pending.question.options[0].coordinates = { [root]: "fixed", "<workspace>": "collision" };
    process.stdout.write(JSON.stringify(success("session.show", collision)));
  } else {
    if (requested !== "session-desk-1") process.exitCode = 8;
    process.stdout.write(JSON.stringify(success("session.show", pending)));
  }
} else if (args[0] === "session" && args[1] === "answer") {
  const option = args[args.indexOf("--option") + 1];
  const command = "session.answer";
  const codes = {
    stale: "session_turn_stale",
    mismatch: "session_axis_mismatch",
    missing: "session_not_found",
    corrupt: "session_corrupt",
    io: "session_io_failed",
    invalid: "session_invalid_argument"
  };
  if (codes[option]) {
    process.exitCode = 2;
    process.stdout.write(JSON.stringify(failure(command, codes[option])));
  } else {
    const valid =
      args[args.indexOf("--session") + 1] === "session-desk-1" &&
      args[args.indexOf("--turn") + 1] === "3" &&
      args[args.indexOf("--axis") + 1] === "desk.frame" &&
      option === "fixed" &&
      args[args.indexOf("--note") + 1] === "Keep $(whoami) && spaces";
    if (!valid) process.exitCode = 8;
    const answered = {
      ...pending,
      state: "planning",
      answered: [{
        axis: "desk.frame",
        option: "fixed",
        label: "Fixed",
        defaulted: false,
        answeredAtUnixMs: "3"
      }],
      updatedUnixMs: "3"
    };
    delete answered.pending;
    process.stdout.write(JSON.stringify(success(command, answered)));
  }
} else {
  process.exitCode = 9;
}
`);
    const controller = new TactusController({
      executable: process.execPath,
      commandPrefix: [fixture.script],
      emit: () => undefined,
    });
    const view = await controller.open(fixture.root);

    const list = await controller.sessions({ workspaceHandle: view.handle, limit: 25 });
    expect(list.sessions[0]?.pending?.findings[0]?.source).toBe("<workspace>");
    expect(Object.keys(list.sessions[0]?.pending?.question.options[0]?.coordinates ?? {})).toEqual([
      "<workspace>",
    ]);
    expect(JSON.stringify(list)).not.toContain("future");
    expect(JSON.stringify(list)).not.toContain(fixture.root);
    await expect(
      controller.session({ workspaceHandle: view.handle, sessionId: "session-desk-1" }),
    ).resolves.toMatchObject({
      sessionId: "session-desk-1",
      state: "awaiting_answer",
    });
    await expect(
      controller.session({ workspaceHandle: view.handle, sessionId: "session-collision-1" }),
    ).rejects.toMatchObject({
      detail: { code: "redaction_key_collision", category: "internal", retryable: false },
    });
    const staleHandle = "bb665bbe-ece0-40e6-8235-2278635aee84";
    for (const request of [
      () => controller.sessions({ workspaceHandle: staleHandle, limit: 25 }),
      () => controller.session({ workspaceHandle: staleHandle, sessionId: "session-desk-1" }),
      () =>
        controller.answer({
          workspaceHandle: staleHandle,
          sessionId: "session-desk-1",
          turn: "3",
          axis: "desk.frame",
          option: "fixed",
        }),
    ]) {
      await expect(request()).rejects.toMatchObject({
        detail: { code: "workspace_handle_stale", category: "validation", retryable: false },
      });
    }
    await expect(
      controller.answer({
        workspaceHandle: view.handle,
        sessionId: "session-desk-1",
        turn: "3",
        axis: "desk.frame",
        option: "fixed",
        note: "Keep $(whoami) && spaces",
      }),
    ).resolves.toMatchObject({ state: "planning", answered: [{ option: "fixed" }] });

    await expect(sessionFailure(controller, view.handle, "stale")).rejects.toMatchObject({
      detail: { code: "session_turn_stale", category: "validation", retryable: false },
    });
    await expect(sessionFailure(controller, view.handle, "mismatch")).rejects.toMatchObject({
      detail: { category: "validation", retryable: false },
    });
    await expect(sessionFailure(controller, view.handle, "missing")).rejects.toMatchObject({
      detail: { category: "workspace", retryable: false },
    });
    await expect(sessionFailure(controller, view.handle, "corrupt")).rejects.toMatchObject({
      detail: { category: "workspace", retryable: false },
    });
    await expect(sessionFailure(controller, view.handle, "io")).rejects.toMatchObject({
      detail: { category: "workspace", retryable: true },
    });
    await expect(sessionFailure(controller, view.handle, "invalid")).rejects.toMatchObject({
      detail: { category: "validation", retryable: false },
    });
    controller.dispose();
  });
});

function sessionFailure(
  controller: TactusController,
  workspaceHandle: string,
  option: string,
): Promise<unknown> {
  return controller.answer({
    workspaceHandle,
    sessionId: "session-desk-1",
    turn: "3",
    axis: "desk.frame",
    option,
  });
}

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
      scripts: [{ relativePath: ".tactus/scripts/010_main.hs", order: 10, runnable: true }, { relativePath: ".tactus/scripts/Support.hs", runnable: false }],
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
