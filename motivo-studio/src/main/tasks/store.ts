import { randomUUID } from "node:crypto";
import { lstat, mkdir, open, readdir, rename, unlink } from "node:fs/promises";
import { join } from "node:path";
import {
  taskDocumentSchema,
  taskIdSchema,
  taskSummarySchema,
  type TaskDocument,
  type TaskSummary,
} from "../../shared/task-contracts";
import { MainProcessError } from "../errors";

const MAX_DOCUMENT_BYTES = 4 * 1024 * 1024;

export function taskError(message: string, code = "task_invalid"): MainProcessError {
  return new MainProcessError({ code, category: "validation", retryable: false, message });
}

/** Motivo owns only .motivo task data; Tactus configuration and journals stay with Tactus. */
export class TaskStore {
  private queue: Promise<unknown> = Promise.resolve();
  private readonly directory: string;

  constructor(readonly root: string) {
    this.directory = join(root, ".motivo", "tasks");
  }

  private serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.queue.then(operation);
    this.queue = result.catch(() => undefined);
    return result;
  }

  private async prepare(): Promise<void> {
    for (const directory of [join(this.root, ".motivo"), this.directory]) {
      await mkdir(directory, { mode: 0o700 }).catch((error: unknown) => {
        if (!isMissingCode(error, "EEXIST")) throw error;
      });
      const info = await lstat(directory);
      if (!info.isDirectory() || info.isSymbolicLink()) {
        throw taskError("Motivo data directories must be ordinary workspace directories.");
      }
    }
  }

  async list(): Promise<TaskSummary[]> {
    return this.serialize(async () => {
      await this.prepare();
      const names = (await readdir(this.directory)).filter((name) => name.endsWith(".json"));
      if (names.length > 1000)
        throw taskError("Too many task files; archive older .motivo/tasks entries.");
      const tasks: TaskSummary[] = [];
      for (const name of names) {
        const id = name.slice(0, -5);
        if (!taskIdSchema.safeParse(id).success) continue;
        const { goal, provider, status, updatedAt, calls } = await this.read(id);
        tasks.push(taskSummarySchema.parse({ id, goal, provider, status, updatedAt, calls }));
      }
      return tasks
        .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))
        .slice(0, 50);
    });
  }

  async get(id: string): Promise<TaskDocument> {
    return this.serialize(async () => {
      await this.prepare();
      return this.read(id);
    });
  }

  async create(document: TaskDocument): Promise<TaskDocument> {
    return this.serialize(async () => {
      await this.prepare();
      try {
        await lstat(this.path(document.id));
      } catch (error) {
        if (!isMissingCode(error, "ENOENT")) throw error;
        return this.write(document);
      }
      throw taskError("This task already exists.");
    });
  }

  async update(id: string, change: (current: TaskDocument) => TaskDocument): Promise<TaskDocument> {
    return this.serialize(async () => {
      await this.prepare();
      const current = await this.read(id);
      const next = change(current);
      if (next.id !== id) throw taskError("A task update cannot change its identity.");
      return this.write({
        ...next,
        revision: current.revision + 1,
        updatedAt: new Date().toISOString(),
      });
    });
  }

  private path(id: string): string {
    return join(this.directory, taskIdSchema.parse(id) + ".json");
  }

  private async read(id: string): Promise<TaskDocument> {
    const contents = await readOrdinaryFile(this.path(id), MAX_DOCUMENT_BYTES);
    let parsed: unknown;
    try {
      parsed = JSON.parse(contents);
    } catch {
      throw taskError("The saved task is invalid; its file was preserved.");
    }
    const result = taskDocumentSchema.safeParse(parsed);
    if (!result.success || result.data.id !== id)
      throw taskError("The saved task is invalid; its file was preserved.");
    return result.data;
  }

  private async write(document: TaskDocument): Promise<TaskDocument> {
    const valid = taskDocumentSchema.parse(document);
    const contents = JSON.stringify(valid, null, 2) + "\n";
    if (Buffer.byteLength(contents) > MAX_DOCUMENT_BYTES) {
      throw taskError(
        "This task reached its history limit. Start a new task with a concise handoff.",
      );
    }
    const temporary = join(this.directory, "." + randomUUID() + ".tmp");
    try {
      const file = await open(temporary, "wx", 0o600);
      try {
        await file.writeFile(contents, "utf8");
        await file.sync();
      } finally {
        await file.close();
      }
      await rename(temporary, this.path(valid.id));
    } finally {
      await unlink(temporary).catch(() => undefined);
    }
    return valid;
  }
}

export async function readOrdinaryFile(path: string, limit: number): Promise<string> {
  const info = await lstat(path);
  if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1 || info.size > limit) {
    throw taskError("Motivo expects a bounded, ordinary local file.");
  }
  const file = await open(path, "r");
  try {
    const actual = await file.stat();
    if (actual.dev !== info.dev || actual.ino !== info.ino || actual.size > limit) {
      throw taskError("The Motivo file changed while it was being opened.");
    }
    const buffer = Buffer.alloc(limit + 1);
    let bytesRead = 0;
    while (bytesRead < buffer.length) {
      const result = await file.read(buffer, bytesRead, buffer.length - bytesRead, bytesRead);
      if (!result.bytesRead) break;
      bytesRead += result.bytesRead;
    }
    if (bytesRead > limit) throw taskError("The Motivo file exceeds its size limit.");
    return new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, bytesRead));
  } finally {
    await file.close();
  }
}

export function isMissingCode(error: unknown, code: string): boolean {
  return typeof error === "object" && error !== null && "code" in error && error.code === code;
}
