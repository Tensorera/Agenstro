import { describe, expect, it, vi } from "vitest";
import { TerminalWriteQueue } from "../../src/renderer/features/terminal-write-queue";

describe("renderer terminal input queue", () => {
  it("serializes input and invalidates queued writes on reset", async () => {
    const queue = new TerminalWriteQueue();
    let release: (() => void) | undefined;
    const first = new Promise<void>((resolve) => {
      release = resolve;
    });
    const write = vi.fn().mockReturnValueOnce(first).mockResolvedValue(undefined);
    queue.enqueue("terminal-1", "first", write, vi.fn());
    queue.enqueue("terminal-1", "second", write, vi.fn());
    await vi.waitFor(() => expect(write).toHaveBeenCalledOnce());
    queue.reset();
    release?.();
    await Promise.resolve();
    expect(write).toHaveBeenCalledOnce();
  });

  it("rejects input once the pending byte budget is full", () => {
    const queue = new TerminalWriteQueue();
    const never = () => new Promise<void>(() => undefined);
    expect(queue.enqueue("terminal-1", "x".repeat(262_145), never, vi.fn())).toBe(false);
  });
});
