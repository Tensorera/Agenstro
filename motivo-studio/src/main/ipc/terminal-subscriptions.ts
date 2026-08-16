import {
  LIMITS,
  terminalStreamMessageSchema,
  type Sequence,
  type SubscriptionId,
  type TerminalId,
} from "../../shared/contracts";
import { IPC } from "../../shared/ipc";
import type { PtyEvent } from "../../shared/pty-protocol";
import type { PtyBroker } from "../pty/pty-broker";
import { StudioFault } from "../errors";
import type { EventSender } from "./run-subscriptions";

interface BufferedOutput {
  readonly sequence: bigint;
  readonly event: Extract<PtyEvent, { kind: "output" }>;
}

interface TerminalOwner {
  readonly owner: EventSender;
  readonly buffered: BufferedOutput[];
  bufferedBytes: number;
  highestSequence: bigint;
  highestDelivered: bigint;
  lastAcknowledged: bigint;
  subscriptionId?: SubscriptionId;
}

export class TerminalSubscriptionManager {
  private readonly terminals = new Map<TerminalId, TerminalOwner>();
  private readonly subscriptions = new Map<SubscriptionId, TerminalId>();
  private readonly stopListening: () => void;

  constructor(private readonly broker: PtyBroker) {
    this.stopListening = broker.onEvent((event) => this.onEvent(event));
  }

  register(owner: EventSender, terminalId: TerminalId): void {
    this.terminals.set(terminalId, {
      owner,
      buffered: [],
      bufferedBytes: 0,
      highestSequence: 0n,
      highestDelivered: 0n,
      lastAcknowledged: 0n,
    });
  }

  assertOwner(ownerId: number, terminalId: TerminalId): void {
    this.owned(ownerId, terminalId);
  }

  subscribe(owner: EventSender, subscriptionId: SubscriptionId, terminalId: TerminalId): void {
    const terminal = this.owned(owner.id, terminalId);
    if (terminal.subscriptionId || this.subscriptions.has(subscriptionId)) {
      throw terminalFault(
        "TERMINAL_SUBSCRIPTION_EXISTS",
        "The terminal already has a subscription.",
      );
    }
    const ownerCount = [...this.terminals.values()].filter(
      (candidate) => candidate.owner.id === owner.id && candidate.subscriptionId !== undefined,
    ).length;
    if (ownerCount >= LIMITS.subscriptionsPerWindow) {
      throw terminalFault("SUBSCRIPTION_LIMIT", "The window subscription limit was reached.");
    }
    terminal.subscriptionId = subscriptionId;
    this.subscriptions.set(subscriptionId, terminalId);
    terminal.buffered.forEach(({ event }) => this.sendOutput(terminal, event));
  }

  acknowledge(ownerId: number, terminalId: TerminalId, sequence: Sequence): void {
    const terminal = this.owned(ownerId, terminalId);
    const acknowledged = BigInt(sequence);
    if (acknowledged < terminal.lastAcknowledged || acknowledged > terminal.highestDelivered) {
      throw terminalFault(
        "TERMINAL_ACK_OUT_OF_RANGE",
        "The terminal acknowledgement is not within delivered output.",
      );
    }
    while (terminal.buffered[0] && terminal.buffered[0].sequence <= acknowledged) {
      const output = terminal.buffered.shift();
      if (output) terminal.bufferedBytes -= output.event.bytes;
    }
    terminal.lastAcknowledged = acknowledged;
    this.broker.acknowledge(terminalId, sequence);
  }

  unsubscribe(ownerId: number, subscriptionId: SubscriptionId): void {
    const terminalId = this.subscriptions.get(subscriptionId);
    if (!terminalId) return;
    const terminal = this.owned(ownerId, terminalId);
    delete terminal.subscriptionId;
    this.subscriptions.delete(subscriptionId);
  }

  close(ownerId: number, terminalId: TerminalId): void {
    const terminal = this.owned(ownerId, terminalId);
    if (terminal.subscriptionId) this.subscriptions.delete(terminal.subscriptionId);
    this.terminals.delete(terminalId);
    this.broker.close(terminalId);
  }

  closeOwner(ownerId: number): void {
    [...this.terminals.entries()]
      .filter(([, terminal]) => terminal.owner.id === ownerId)
      .forEach(([terminalId, terminal]) => {
        if (terminal.subscriptionId) this.subscriptions.delete(terminal.subscriptionId);
        this.terminals.delete(terminalId);
        this.broker.close(terminalId);
      });
  }

  shutdown(): void {
    this.stopListening();
    this.subscriptions.clear();
    this.terminals.clear();
  }

  private onEvent(event: PtyEvent): void {
    if (event.kind === "created" || event.kind === "error") return;
    const terminal = this.terminals.get(event.terminalId);
    if (!terminal || terminal.owner.isDestroyed()) return;
    if (event.kind === "output") {
      const sequence = BigInt(event.sequence);
      if (sequence !== terminal.highestSequence + 1n) {
        this.broker.close(event.terminalId);
        return;
      }
      if (terminal.bufferedBytes + event.bytes > LIMITS.runBatchBytes * 2) {
        this.broker.close(event.terminalId);
        return;
      }
      terminal.highestSequence = sequence;
      terminal.buffered.push({ sequence, event });
      terminal.bufferedBytes += event.bytes;
      if (terminal.subscriptionId) this.sendOutput(terminal, event);
      return;
    }
    if (terminal.subscriptionId) {
      terminal.owner.send(
        IPC.terminalsEvent,
        terminalStreamMessageSchema.parse({
          kind: "exit",
          terminalId: event.terminalId,
          exitCode: event.exitCode,
          reason: event.reason,
        }),
      );
      this.subscriptions.delete(terminal.subscriptionId);
    }
    this.terminals.delete(event.terminalId);
  }

  private sendOutput(terminal: TerminalOwner, event: Extract<PtyEvent, { kind: "output" }>): void {
    terminal.owner.send(
      IPC.terminalsEvent,
      terminalStreamMessageSchema.parse({
        kind: "output",
        terminalId: event.terminalId,
        sequence: event.sequence,
        data: event.data,
      }),
    );
    terminal.highestDelivered = BigInt(event.sequence);
  }

  private owned(ownerId: number, terminalId: TerminalId): TerminalOwner {
    const terminal = this.terminals.get(terminalId);
    if (!terminal || terminal.owner.id !== ownerId) {
      throw terminalFault("TERMINAL_NOT_FOUND", "The terminal is not owned by this window.");
    }
    return terminal;
  }
}

function terminalFault(code: string, message: string): StudioFault {
  return new StudioFault({ code, category: "resource", retryable: false, message });
}
