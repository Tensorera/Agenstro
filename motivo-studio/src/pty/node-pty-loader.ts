import { readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { createRequire } from "node:module";
import { z } from "zod";
import type { IPty, IPtyForkOptions } from "node-pty";

export type PtySpawn = (file: string, args: string[], options: IPtyForkOptions) => IPty;

const packageSchema = z
  .object({ name: z.literal("node-pty"), version: z.literal("1.1.0") })
  .passthrough();

export function loadNodePty(root: string): PtySpawn {
  if (!isAbsolute(root)) throw new Error("The node-pty package root must be absolute.");
  const packagePath = join(root, "package.json");
  packageSchema.parse(JSON.parse(readFileSync(packagePath, "utf8")));
  const localRequire = createRequire(packagePath);
  const candidate: unknown = localRequire(root);
  if (!isRecord(candidate) || typeof candidate.spawn !== "function") {
    throw new Error("The locked node-pty module has an invalid export surface.");
  }
  const spawn = candidate.spawn;
  return (file, args, options) => {
    const terminal: unknown = Reflect.apply(spawn, candidate, [file, args, options]);
    if (!isPty(terminal)) throw new Error("node-pty returned an invalid terminal handle.");
    return terminal;
  };
}

function isPty(value: unknown): value is IPty {
  return (
    isRecord(value) &&
    typeof value.write === "function" &&
    typeof value.resize === "function" &&
    typeof value.kill === "function" &&
    typeof value.pause === "function" &&
    typeof value.resume === "function" &&
    typeof value.onData === "function" &&
    typeof value.onExit === "function"
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
