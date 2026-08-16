import { describe, expect, it, vi } from "vitest";
import type { PtyBroker } from "../../src/main/pty/pty-broker";
import { TerminalSubscriptionManager } from "../../src/main/ipc/terminal-subscriptions";
import type { PtyEvent } from "../../src/shared/pty-protocol";
import {
  sequenceSchema,
  subscriptionIdSchema,
  terminalIdSchema,
  type TerminalId,
} from "../../src/shared/contracts";
import type { EventSender } from "../../src/main/ipc/run-subscriptions";

describe("terminal subscription accounting", () => {
  it("rejects future or regressing acknowledgements and closes on a sequence gap", () => {
    let listener: ((event: PtyEvent) => void) | undefined;
    const acknowledge = vi.fn();
    const close = vi.fn();
    const broker = {
      onEvent: vi.fn((value: (event: PtyEvent) => void) => {
        listener = value;
        return vi.fn();
      }),
      acknowledge,
      close,
    } as unknown as PtyBroker;
    const sender: EventSender = {
      id: 4,
      isDestroyed: () => false,
      send: vi.fn(),
    };
    const terminalId = terminalIdSchema.parse("c345728d-860a-4646-93fe-0e9c68e9ad34");
    const manager = new TerminalSubscriptionManager(broker);
    manager.register(sender, terminalId);
    listener?.(output(terminalId, 1));
    expect(() => manager.acknowledge(4, terminalId, sequenceSchema.parse("1"))).toThrow(
      /acknowledgement/,
    );
    manager.subscribe(
      sender,
      subscriptionIdSchema.parse("e7f67ecf-7471-427c-96f6-2a899f26e139"),
      terminalId,
    );

    expect(() => manager.acknowledge(4, terminalId, sequenceSchema.parse("2"))).toThrow(
      /acknowledgement/,
    );
    manager.acknowledge(4, terminalId, sequenceSchema.parse("1"));
    expect(acknowledge).toHaveBeenCalledOnce();
    expect(() => manager.acknowledge(4, terminalId, sequenceSchema.parse("0"))).toThrow(
      /acknowledgement/,
    );

    listener?.(output(terminalId, 3));
    expect(close).toHaveBeenCalledWith(terminalId);
  });
});

function output(terminalId: TerminalId, sequence: number): PtyEvent {
  return {
    kind: "output",
    terminalId,
    sequence: sequenceSchema.parse(String(sequence)),
    data: "x",
    bytes: 1,
  };
}
