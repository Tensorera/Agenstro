import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { TaskService } from "../../src/main/tasks/service";
import { invokeProvider } from "../../src/main/tasks/transport";
import { taskDocumentSchema } from "../../src/shared/task-contracts";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const tactus = resolve(
  process.env.MOTIVO_TEST_TACTUS_BIN ??
    join(
      repository,
      "Build",
      "cargo",
      "debug",
      process.platform === "win32" ? "tactus.exe" : "tactus",
    ),
);
const available = existsSync(tactus);
const roots: string[] = [];
const services: TaskService[] = [];

afterEach(async () => {
  for (const service of services.splice(0)) {
    service.dispose();
    await service.waitForIdle();
  }
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

function service(): TaskService {
  // Pin the real transport to the selected Rust binary; no mocked invocation.
  const result = new TaskService({
    invoke: (input) => invokeProvider(input, { executable: tactus, timeoutMs: 20_000 }),
  });
  services.push(result);
  return result;
}

function command(root: string, args: readonly string[]): string {
  return execFileSync(tactus, [...args], {
    cwd: root,
    encoding: "utf8",
    timeout: 20_000,
    windowsHide: true,
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 4 * 1024 * 1024,
  });
}

// A deterministic provider fixture authors this ordinary project capability.
// This acceptance test exercises orchestration and evidence, not model ability.
const checkerSource = String.raw`
const fs = require("node:fs");
const request = JSON.parse(fs.readFileSync(0, "utf8"));
const numbers = request.params.numbers;
const valid = request.api === "agenstro.plugin/v1" && request.method === "summarize" &&
  Array.isArray(numbers) && numbers.every(value => typeof value === "number" && Number.isFinite(value));
const result = valid
  ? {type:"result", id:request.id, ok:true, value:{sum:numbers.reduce((sum, value) => sum + value, 0), count:numbers.length}}
  : {type:"result", id:request.id, ok:false, error:{code:"invalid_input", message:"summarize requires finite numbers"}};
process.stdout.write(JSON.stringify(result) + "\n");
if (!valid) process.exitCode = 1;
`;

const providerSource =
  "const CHECKER_SOURCE = " +
  JSON.stringify(checkerSource) +
  ";\n" +
  String.raw`
const fs = require("node:fs");
const path = require("node:path");
const {execFileSync} = require("node:child_process");
const request = JSON.parse(fs.readFileSync(0, "utf8"));
const root = request.params.workspace;
const tactus = process.argv[2];
const writeJson = (relative, value) => fs.writeFileSync(path.join(root, relative), JSON.stringify(value, null, 2) + "\n");
const readJson = relative => JSON.parse(fs.readFileSync(path.join(root, relative), "utf8"));
const emit = value => process.stdout.write(JSON.stringify(value) + "\n");
try {
  if (request.api !== "agenstro.plugin/v1" || request.method !== "invoke") throw new Error("Expected a provider invocation");
  const block = request.params.prompt.split("\n\n").find(part => part.startsWith('{"goal":'));
  if (!block) throw new Error("No Motivo task handoff supplied");
  const context = JSON.parse(block);
  const previous = context.recentHistory.at(-1)?.report;
  let report = {
    action:"investigate", focus:"A small local numeric observation capability",
    summary:"", findings:[], unknowns:[], decision:"", artifacts:[], checks:[], next:"", status:"continue"
  };
  if (!previous) {
    const input = readJson("numbers.json");
    report.summary = "Found numeric input without a project summary plugin.";
    report.findings = [{statement:"Input contains " + input.length + " numbers.", source:"numbers.json"}];
    report.decision = "Add a small number-summary plugin and fixtures through Tactus.";
    report.next = "Implement the project plugin and execute its fixtures.";
  } else if (previous.action === "investigate") {
    const directory = path.join(root, "plugins", "number-summary");
    fs.mkdirSync(directory, {recursive:true});
    fs.mkdirSync(path.join(root, "artifacts"), {recursive:true});
    const pluginPath = path.join(directory, "index.cjs");
    fs.writeFileSync(pluginPath, CHECKER_SOURCE);
    fs.appendFileSync(path.join(root, ".tactus", "tactus.toml"),
      "\n[plugins.number-summary]\ncommand = " + JSON.stringify([process.execPath, pluginPath]) + "\n");
    const cases = [
      {numbers:readJson("numbers.json"), expected:{sum:6, count:3}},
      {numbers:[], expected:{sum:0, count:0}},
      {numbers:[-3, 5], expected:{sum:2, count:2}}
    ];
    writeJson("plugins/number-summary/fixtures.json", cases);
    const results = cases.map(testCase => {
      const output = execFileSync(tactus, ["plugin-call", "number-summary", "summarize",
        "--namespace", "plugin", "--root", root, "--params", JSON.stringify({numbers:testCase.numbers}), "--json"],
        {cwd:root, encoding:"utf8", timeout:10000, windowsHide:true});
      const invocation = JSON.parse(output);
      if (invocation.summary.outcome.kind !== "succeeded") throw new Error("Plugin invocation did not succeed");
      const observed = invocation.summary.outcome.terminal.value;
      return {...testCase, observed, passed:observed.sum === testCase.expected.sum && observed.count === testCase.expected.count};
    });
    if (results.some(result => !result.passed)) throw new Error("Project plugin fixtures failed");
    writeJson("artifacts/plugin-check.json", {checkedVia:"tactus plugin-call", cases:results});
    report.action = "try";
    report.summary = "Created the plugin and ran three fixtures through Tactus.";
    report.decision = "Keep the tested implementation and review the recorded evidence.";
    report.artifacts = ["plugins/number-summary/index.cjs", "plugins/number-summary/fixtures.json", "artifacts/plugin-check.json"];
    report.checks = [{name:"number-summary fixtures", result:"passed", detail:"Three real Tactus plugin calls matched the fixture expectations.", source:"artifacts/plugin-check.json"}];
    report.next = "Integrate the saved checks into the final delivery.";
  } else if (previous.action === "try") {
    if (previous.checks[0]?.result !== "passed" || readJson("artifacts/plugin-check.json").cases.length !== 3)
      throw new Error("The persisted handoff or check artifact is missing");
    fs.writeFileSync(path.join(root, "artifacts", "delivery.md"), "Number summary plugin delivered with three checked fixtures.\n");
    report.action = "integrate";
    report.summary = "Integrated the saved checks and delivered the project plugin.";
    report.decision = "The requested local capability and its fixtures are ready.";
    report.artifacts = [...previous.artifacts, "artifacts/delivery.md"];
    report.checks = previous.checks;
    report.status = "completed";
  } else throw new Error("Unexpected handoff; this fixture does not improvise task progress");
  emit({type:"result", id:request.id, ok:true, value:{text:JSON.stringify(report)}});
} catch (error) {
  emit({type:"result", id:request.id, ok:false, error:{code:"fixture_failed", message:error.message}});
  process.exitCode = 1;
}
`;

describe.skipIf(!available)(
  available
    ? "Motivo task through real Tactus"
    : "Motivo task through real Tactus (binary missing: set MOTIVO_TEST_TACTUS_BIN or cargo build -p tactus-runtime --bin tactus)",
  () => {
    it("creates and checks a project plugin without Haskell, then resumes its persisted handoff", async () => {
      const temporary = await mkdtemp(join(tmpdir(), "motivo-real-tactus-"));
      roots.push(temporary);
      const root = join(temporary, "task workspace 中文");
      await mkdir(root);
      command(root, ["init", root, "--sdk", join(repository, "clef-sdk")]);
      const providerPath = join(temporary, "provider.cjs");
      await writeFile(providerPath, providerSource);
      await writeFile(join(root, "numbers.json"), "[1, 2, 3]\n");
      const configPath = join(root, ".tactus", "tactus.toml");
      const initialConfig = await readFile(configPath, "utf8");
      await writeFile(
        configPath,
        initialConfig.replace('default_provider = "codex"', 'default_provider = "fixture"') +
          "\n[providers.fixture]\ncommand = " +
          JSON.stringify([process.execPath, providerPath, tactus]) +
          "\n",
      );

      const workspaceHandle = randomUUID();
      const first = service();
      const created = await first.create(root, {
        workspaceHandle,
        goal: "Add a project-local number summary plugin and check its fixtures.",
        constraints:
          "Use Tactus for provider and plugin calls. No Haskell generation or live model calls.",
        provider: "fixture",
      });
      await first.continue(root, { workspaceHandle, taskId: created.id, maxCalls: 2 });
      await first.waitForIdle();
      const paused = await first.current(root, created.id);
      expect(paused.status, paused.message).toBe("paused");
      expect(paused.calls).toBe(2);
      expect(paused.rounds.map((round) => round.report?.action)).toEqual(["investigate", "try"]);
      expect(await readFile(join(root, "plugins/number-summary/index.cjs"), "utf8")).toBe(
        checkerSource,
      );
      const checks = JSON.parse(await readFile(join(root, "artifacts/plugin-check.json"), "utf8"));
      expect(checks).toEqual({
        checkedVia: "tactus plugin-call",
        cases: [
          {
            numbers: [1, 2, 3],
            expected: { sum: 6, count: 3 },
            observed: { sum: 6, count: 3 },
            passed: true,
          },
          {
            numbers: [],
            expected: { sum: 0, count: 0 },
            observed: { sum: 0, count: 0 },
            passed: true,
          },
          {
            numbers: [-3, 5],
            expected: { sum: 2, count: 2 },
            observed: { sum: 2, count: 2 },
            passed: true,
          },
        ],
      });
      expect(
        JSON.parse(await readFile(join(root, "plugins/number-summary/fixtures.json"), "utf8")),
      ).toHaveLength(3);
      first.dispose();

      const restored = service();
      expect(await restored.current(root, created.id)).toEqual(paused);
      await restored.continue(root, {
        workspaceHandle,
        taskId: created.id,
        maxCalls: 1,
        note: "Integrate the saved plugin checks and finish.",
      });
      await restored.waitForIdle();
      const completed = await restored.current(root, created.id);
      expect(completed.status, completed.message).toBe("completed");
      expect(completed.calls).toBe(3);
      expect(completed.rounds.map((round) => round.report?.action)).toEqual([
        "investigate",
        "try",
        "integrate",
      ]);
      expect(
        completed.rounds.every(
          (round) => round.outcome === "succeeded" && round.elapsedMs !== undefined,
        ),
      ).toBe(true);
      expect(await readFile(join(root, "artifacts/delivery.md"), "utf8")).toContain(
        "three checked fixtures",
      );
      const saved = taskDocumentSchema.parse(
        JSON.parse(await readFile(join(root, ".motivo/tasks", created.id + ".json"), "utf8")),
      );
      expect(saved).toEqual(completed);
      expect(await service().current(root, created.id)).toEqual(completed);
      expect(await readdir(join(root, ".tactus/scripts"))).toEqual([]);

      // Independently exercise the delivered capability on a previously unseen input.
      const probe = JSON.parse(
        command(root, [
          "plugin-call",
          "number-summary",
          "summarize",
          "--namespace",
          "plugin",
          "--root",
          root,
          "--params",
          JSON.stringify({ numbers: [10, -7] }),
          "--json",
        ]),
      );
      expect(probe.summary.outcome.kind).toBe("succeeded");
      expect(probe.summary.outcome.terminal.value).toEqual({ sum: 3, count: 2 });
      const runs = join(root, ".tactus/runs");
      const runIds = await readdir(runs);
      expect(runIds.length).toBeGreaterThanOrEqual(7); // Three agents, three fixtures, one independent check.
      for (const runId of runIds) {
        const summary = JSON.parse(await readFile(join(runs, runId, "summary.json"), "utf8"));
        expect(summary.api).toBe("agenstro.trace/v1");
        expect(summary.outcome.kind).toBe("succeeded");
        expect(await readFile(join(runs, runId, "events.jsonl"), "utf8")).toContain(
          "runtime.state_transition",
        );
      }
    }, 60_000);
  },
);
