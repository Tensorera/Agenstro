import { describe, expect, it } from "vitest";
import { LIMITS } from "../../src/shared/contracts";
import {
  filePageInputSchema,
  fileSaveInputSchema,
  runSubscribeRequestSchema,
  terminalCreateInputSchema,
  terminalWriteInputSchema,
} from "../../src/shared/ipc";

describe("bounded IPC contracts", () => {
  it("rejects unknown keys and oversized pages", () => {
    expect(() =>
      filePageInputSchema.parse({
        workspaceId: "workspace-1",
        parentId: "entry-root",
        pageSize: LIMITS.filePage + 1,
      }),
    ).toThrow();
    expect(() =>
      runSubscribeRequestSchema.parse({
        runId: "run-1",
        afterSequence: "0",
        channel: "arbitrary:invoke",
      }),
    ).toThrow();
  });

  it("applies UTF-8 byte limits instead of JavaScript character counts", () => {
    const oversized = "\u754c".repeat(Math.floor(LIMITS.terminalInputBytes / 3) + 1);
    expect(() =>
      terminalWriteInputSchema.parse({ terminalId: "terminal-1", data: oversized }),
    ).toThrow();
    expect(() =>
      fileSaveInputSchema.parse({
        requestId: "d7ef7a0c-63c6-4f33-8312-8a0c463f675d",
        workspaceId: "workspace-1",
        entryId: "entry-1",
        expectedRevision: "revision-1",
        content: "ok",
      }),
    ).not.toThrow();
  });

  it("allows only declared shell profiles and bounded terminal geometry", () => {
    expect(() =>
      terminalCreateInputSchema.parse({
        workspaceId: "workspace-1",
        profileId: "cmd-with-arbitrary-args",
        cols: 80,
        rows: 24,
      }),
    ).toThrow();
    expect(() =>
      terminalCreateInputSchema.parse({
        workspaceId: "workspace-1",
        profileId: "powershell",
        cols: 501,
        rows: 24,
      }),
    ).toThrow();
  });
});
