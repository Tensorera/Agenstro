import { utf8Bytes } from "../../shared/contracts";

const MAX_PENDING_INPUT_BYTES = 262_144;

interface PendingWrite {
  readonly terminalId: string;
  readonly data: string;
  readonly bytes: number;
}

export class TerminalWriteQueue {
  private readonly pending: PendingWrite[] = [];
  private pendingBytes = 0;
  private writing = false;
  private generation = 0;

  enqueue(
    terminalId: string,
    data: string,
    write: (input: { terminalId: string; data: string }) => Promise<void>,
    onError: (error: unknown) => void,
  ): boolean {
    const bytes = utf8Bytes(data);
    if (this.pendingBytes + bytes > MAX_PENDING_INPUT_BYTES) return false;
    this.pending.push({ terminalId, data, bytes });
    this.pendingBytes += bytes;
    if (!this.writing) void this.drain(this.generation, write, onError);
    return true;
  }

  reset(): void {
    this.generation += 1;
    this.pending.length = 0;
    this.pendingBytes = 0;
    this.writing = false;
  }

  private async drain(
    generation: number,
    write: (input: { terminalId: string; data: string }) => Promise<void>,
    onError: (error: unknown) => void,
  ): Promise<void> {
    this.writing = true;
    while (generation === this.generation) {
      const next = this.pending.shift();
      if (!next) break;
      this.pendingBytes -= next.bytes;
      try {
        await write({ terminalId: next.terminalId, data: next.data });
      } catch (error) {
        if (generation === this.generation) onError(error);
        break;
      }
    }
    if (generation === this.generation) this.writing = false;
  }
}
