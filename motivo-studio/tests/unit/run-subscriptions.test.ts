import { describe, expect, it, vi } from "vitest";
import type { DaemonClient, RunObserver } from "../../src/main/daemon/daemon-client";
import { RunSubscriptionManager, type EventSender } from "../../src/main/ipc/run-subscriptions";
import {
  runEventSchema,
  runStreamMessageSchema,
  sequenceSchema,
  subscriptionIdSchema,
} from "../../src/shared/contracts";
import { IPC } from "../../src/shared/ipc";

describe("resumable run subscriptions", () => {
  it("starts after the requested sequence, acknowledges progress, and fails closed on a gap", () => {
    let observer: RunObserver | undefined;
    const cancel = vi.fn();
    const daemon = {
      watchRun: vi.fn((_runId: string, _after: string, value: RunObserver) => {
        observer = value;
        return { cancel };
      }),
    } as unknown as DaemonClient;
    const sent: Array<{ channel: string; message: unknown }> = [];
    const sender: EventSender = {
      id: 7,
      isDestroyed: () => false,
      send: (channel, message) => sent.push({ channel, message }),
    };
    const manager = new RunSubscriptionManager(daemon);
    const subscriptionId = subscriptionIdSchema.parse("8f42cd1f-ec8e-4ce0-934d-e06f18f623a7");
    manager.subscribe(sender, {
      subscriptionId,
      runId: "run-1",
      afterSequence: sequenceSchema.parse("4"),
    });

    observer?.onEvent(event(5));
    expect(sent[0]?.channel).toBe(IPC.runsEvent);
    expect(runStreamMessageSchema.parse(sent[0]?.message)).toMatchObject({ kind: "events" });
    manager.acknowledge(7, subscriptionId, sequenceSchema.parse("5"));
    expect(() => manager.acknowledge(7, subscriptionId, sequenceSchema.parse("4"))).toThrow(
      /acknowledgement/,
    );
    expect(() => manager.acknowledge(7, subscriptionId, sequenceSchema.parse("6"))).toThrow(
      /acknowledgement/,
    );

    observer?.onEvent(event(7));
    expect(runStreamMessageSchema.parse(sent[1]?.message)).toMatchObject({
      kind: "resync-required",
      reason: "gap",
      lastSafeSequence: "5",
    });
    expect(cancel).toHaveBeenCalledOnce();
  });
});

function event(sequence: number) {
  return runEventSchema.parse({
    runId: "run-1",
    sequence: String(sequence),
    occurredAt: "2026-08-01T12:00:00.000Z",
    body: { kind: "stage", stageId: "stage-1", label: "Execute", state: "running" },
  });
}
