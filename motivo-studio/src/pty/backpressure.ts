export const PTY_OUTPUT_LIMITS = {
  frameBytes: 65_536,
  highWaterBytes: 524_288,
  lowWaterBytes: 262_144,
  hardLimitBytes: 1_048_576,
} as const;

export function splitUtf8(value: string, maximumBytes = PTY_OUTPUT_LIMITS.frameBytes): string[] {
  if (!Number.isInteger(maximumBytes) || maximumBytes < 4) {
    throw new RangeError("A UTF-8 chunk limit must be an integer of at least four bytes.");
  }
  const chunks: string[] = [];
  let current = "";
  let currentBytes = 0;
  for (const character of value) {
    const bytes = Buffer.byteLength(character, "utf8");
    if (currentBytes + bytes > maximumBytes && current.length > 0) {
      chunks.push(current);
      current = "";
      currentBytes = 0;
    }
    current += character;
    currentBytes += bytes;
  }
  if (current.length > 0) chunks.push(current);
  return chunks;
}

interface PendingChunk {
  readonly sequence: number;
  readonly bytes: number;
}

export class OutputWindow {
  private readonly pending: PendingChunk[] = [];
  private bytes = 0;
  private pausedValue = false;

  get pendingBytes(): number {
    return this.bytes;
  }

  get paused(): boolean {
    return this.pausedValue;
  }

  record(sequence: number, bytes: number): "continue" | "pause" | "terminate" {
    if (!Number.isSafeInteger(sequence) || sequence < 1 || !Number.isInteger(bytes) || bytes < 0) {
      throw new RangeError("PTY output accounting values are invalid.");
    }
    if (this.bytes + bytes > PTY_OUTPUT_LIMITS.hardLimitBytes) {
      return "terminate";
    }
    this.pending.push({ sequence, bytes });
    this.bytes += bytes;
    if (!this.pausedValue && this.bytes >= PTY_OUTPUT_LIMITS.highWaterBytes) {
      this.pausedValue = true;
      return "pause";
    }
    return "continue";
  }

  acknowledge(sequence: number): "continue" | "resume" {
    if (!Number.isSafeInteger(sequence) || sequence < 0) {
      throw new RangeError("PTY acknowledgement sequence is invalid.");
    }
    while (this.pending[0] && this.pending[0].sequence <= sequence) {
      const acknowledged = this.pending.shift();
      if (acknowledged) this.bytes -= acknowledged.bytes;
    }
    if (this.pausedValue && this.bytes <= PTY_OUTPUT_LIMITS.lowWaterBytes) {
      this.pausedValue = false;
      return "resume";
    }
    return "continue";
  }
}
