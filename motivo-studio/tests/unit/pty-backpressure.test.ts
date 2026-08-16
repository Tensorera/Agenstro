import { describe, expect, it } from "vitest";
import { OutputWindow, PTY_OUTPUT_LIMITS, splitUtf8 } from "../../src/pty/backpressure";

describe("PTY output backpressure", () => {
  it("splits UTF-8 without breaking code points or frame limits", () => {
    const input = `${"a".repeat(PTY_OUTPUT_LIMITS.frameBytes - 2)}\u754c\ud83d\ude80tail`;
    const chunks = splitUtf8(input);
    expect(chunks.join("")).toBe(input);
    expect(
      chunks.every((chunk) => Buffer.byteLength(chunk, "utf8") <= PTY_OUTPUT_LIMITS.frameBytes),
    ).toBe(true);
  });

  it("pauses at high water, resumes at low water, and terminates before hard overflow", () => {
    const window = new OutputWindow();
    for (let sequence = 1; sequence < 8; sequence += 1) {
      expect(window.record(sequence, PTY_OUTPUT_LIMITS.frameBytes)).toBe("continue");
    }
    expect(window.record(8, PTY_OUTPUT_LIMITS.frameBytes)).toBe("pause");
    expect(window.paused).toBe(true);
    expect(window.acknowledge(4)).toBe("resume");
    expect(window.pendingBytes).toBe(PTY_OUTPUT_LIMITS.lowWaterBytes);

    const saturated = new OutputWindow();
    for (let sequence = 1; sequence <= 16; sequence += 1) {
      saturated.record(sequence, PTY_OUTPUT_LIMITS.frameBytes);
    }
    expect(saturated.record(17, 1)).toBe("terminate");
  });
});
