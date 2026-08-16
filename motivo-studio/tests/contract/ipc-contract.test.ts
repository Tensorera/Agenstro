import { describe, expect, it } from "vitest";
import {
  actionRequestSchema,
  decimalStringSchema,
  LIMITS,
  studioSnapshotSchema,
} from "../../src/shared/contracts";
import { runEventsInputSchema } from "../../src/shared/ipc";

describe("bounded Motivo IPC contracts", () => {
  it("keeps JavaScript-unsafe counters as canonical decimal text", () => {
    expect(decimalStringSchema.parse("18446744073709551615")).toBe("18446744073709551615");
    for (const invalid of ["", "01", "-1", "1.0", " 1", "18446744073709551616"]) {
      expect(() => decimalStringSchema.parse(invalid)).toThrow();
    }
  });

  it("rejects unknown action authority and oversized generation goals", () => {
    expect(() => actionRequestSchema.parse({ kind: "run", root: "C:\\secrets" })).toThrow();
    expect(() =>
      actionRequestSchema.parse({
        kind: "generate",
        goal: "界".repeat(Math.floor(LIMITS.generationGoalBytes / 3) + 1),
      }),
    ).toThrow();
    expect(() =>
      actionRequestSchema.parse({
        kind: "smoke",
        targets: [{ namespace: "provider", name: "codex" }],
        live: false,
      }),
    ).not.toThrow();
  });

  it("accepts only bounded opaque event-page requests", () => {
    expect(() =>
      runEventsInputSchema.parse({ runId: "../events.jsonl", after: "0", limit: 10 }),
    ).toThrow();
    expect(() =>
      runEventsInputSchema.parse({
        runId: "run-123-42-0",
        after: "0",
        limit: LIMITS.eventPage + 1,
      }),
    ).toThrow();
  });

  it("does not admit an absolute workspace path into the Studio snapshot", () => {
    const valid = snapshot();
    expect(studioSnapshotSchema.parse(valid).workspace.name).toBe("sample");
    expect(() =>
      studioSnapshotSchema.parse({
        ...valid,
        workspace: { name: "sample", root: "D:\\private\\sample" },
      }),
    ).toThrow();
  });
});

function snapshot(): Record<string, unknown> {
  return {
    api: "agenstro.studio/v1",
    generatedAtUnixMs: "1770000000000",
    workspace: { name: "sample" },
    health: { ok: true, checks: [] },
    scripts: [],
    registries: { defaultProvider: "codex", providers: [], effects: [], plugins: [] },
    runs: [],
  };
}
