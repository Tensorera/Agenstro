import {
  LIMITS,
  runStreamMessageSchema,
  type RunEvent,
  type Sequence,
  type StudioError,
  type SubscriptionId,
} from "../../shared/contracts";
import { IPC } from "../../shared/ipc";
import type { DaemonClient, RunWatch } from "../daemon/daemon-client";
import { StudioFault } from "../errors";

const MAX_PENDING_EVENTS = 256;

export interface EventSender {
  readonly id: number;
  isDestroyed(): boolean;
  send(channel: string, message: unknown): void;
}

interface PendingEvent {
  readonly sequence: bigint;
  readonly bytes: number;
}

interface Subscription {
  readonly id: SubscriptionId;
  readonly owner: EventSender;
  readonly runId: string;
  readonly pending: PendingEvent[];
  expected: bigint;
  pendingBytes: number;
  lastSafe: Sequence;
  watch?: RunWatch;
}

export class RunSubscriptionManager {
  private readonly subscriptions = new Map<SubscriptionId, Subscription>();

  constructor(private readonly daemon: DaemonClient) {}

  subscribe(
    owner: EventSender,
    input: {
      readonly subscriptionId: SubscriptionId;
      readonly runId: string;
      readonly afterSequence: Sequence;
    },
  ): void {
    if (this.subscriptions.has(input.subscriptionId)) {
      throw subscriptionFault(
        "SUBSCRIPTION_EXISTS",
        "The subscription identifier is already active.",
      );
    }
    const ownerCount = [...this.subscriptions.values()].filter(
      (subscription) => subscription.owner.id === owner.id,
    ).length;
    if (ownerCount >= LIMITS.subscriptionsPerWindow) {
      throw subscriptionFault("SUBSCRIPTION_LIMIT", "The window subscription limit was reached.");
    }
    const subscription: Subscription = {
      id: input.subscriptionId,
      owner,
      runId: input.runId,
      pending: [],
      expected: BigInt(input.afterSequence) + 1n,
      pendingBytes: 0,
      lastSafe: input.afterSequence,
    };
    this.subscriptions.set(input.subscriptionId, subscription);
    subscription.watch = this.daemon.watchRun(input.runId, input.afterSequence, {
      onEvent: (event) => this.onEvent(subscription, event),
      onError: (error) => this.close(subscription, error),
      onEnd: () => this.close(subscription),
    });
  }

  acknowledge(ownerId: number, subscriptionId: SubscriptionId, highestSequence: Sequence): void {
    const subscription = this.owned(ownerId, subscriptionId);
    const acknowledged = BigInt(highestSequence);
    if (acknowledged < BigInt(subscription.lastSafe) || acknowledged >= subscription.expected) {
      throw subscriptionFault("ACK_OUT_OF_RANGE", "The run acknowledgement exceeds sent events.");
    }
    while (subscription.pending[0] && subscription.pending[0].sequence <= acknowledged) {
      const pending = subscription.pending.shift();
      if (pending) subscription.pendingBytes -= pending.bytes;
    }
    subscription.lastSafe = highestSequence;
  }

  unsubscribe(ownerId: number, subscriptionId: SubscriptionId): void {
    const subscription = this.subscriptions.get(subscriptionId);
    if (!subscription) return;
    if (subscription.owner.id !== ownerId) {
      throw subscriptionFault(
        "SUBSCRIPTION_NOT_FOUND",
        "The subscription is not owned by this window.",
      );
    }
    subscription.watch?.cancel();
    this.subscriptions.delete(subscriptionId);
  }

  closeOwner(ownerId: number): void {
    [...this.subscriptions.values()]
      .filter((subscription) => subscription.owner.id === ownerId)
      .forEach((subscription) => {
        subscription.watch?.cancel();
        this.subscriptions.delete(subscription.id);
      });
  }

  closeAll(): void {
    [...this.subscriptions.values()].forEach((subscription) => subscription.watch?.cancel());
    this.subscriptions.clear();
  }

  private onEvent(subscription: Subscription, event: RunEvent): void {
    if (!this.subscriptions.has(subscription.id) || subscription.owner.isDestroyed()) {
      this.subscriptions.delete(subscription.id);
      subscription.watch?.cancel();
      return;
    }
    const sequence = BigInt(event.sequence);
    if (event.runId !== subscription.runId || sequence !== subscription.expected) {
      this.resync(subscription, "gap");
      return;
    }
    const bytes = Buffer.byteLength(JSON.stringify(event), "utf8");
    if (
      subscription.pending.length >= MAX_PENDING_EVENTS ||
      subscription.pendingBytes + bytes > LIMITS.runBatchBytes
    ) {
      this.resync(subscription, "backpressure");
      return;
    }
    subscription.pending.push({ sequence, bytes });
    subscription.pendingBytes += bytes;
    subscription.expected += 1n;
    subscription.owner.send(
      IPC.runsEvent,
      runStreamMessageSchema.parse({
        kind: "events",
        subscriptionId: subscription.id,
        events: [event],
      }),
    );
  }

  private resync(subscription: Subscription, reason: "gap" | "backpressure" | "retention"): void {
    if (!subscription.owner.isDestroyed()) {
      subscription.owner.send(
        IPC.runsEvent,
        runStreamMessageSchema.parse({
          kind: "resync-required",
          subscriptionId: subscription.id,
          lastSafeSequence: subscription.lastSafe,
          reason,
        }),
      );
    }
    subscription.watch?.cancel();
    this.subscriptions.delete(subscription.id);
  }

  private close(subscription: Subscription, error?: StudioError): void {
    if (!this.subscriptions.delete(subscription.id)) return;
    if (!subscription.owner.isDestroyed()) {
      subscription.owner.send(
        IPC.runsEvent,
        runStreamMessageSchema.parse({
          kind: "closed",
          subscriptionId: subscription.id,
          error,
        }),
      );
    }
  }

  private owned(ownerId: number, subscriptionId: SubscriptionId): Subscription {
    const subscription = this.subscriptions.get(subscriptionId);
    if (!subscription || subscription.owner.id !== ownerId) {
      throw subscriptionFault(
        "SUBSCRIPTION_NOT_FOUND",
        "The subscription is not owned by this window.",
      );
    }
    return subscription;
  }
}

function subscriptionFault(code: string, message: string): StudioFault {
  return new StudioFault({ code, category: "resource", retryable: false, message });
}
