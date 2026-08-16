import { isAbsolute, resolve } from "node:path";

const LAUNCH_API = "motivo.launch/v1";
const MAX_PATH_CHARACTERS = 32_767;

export interface LaunchRequest {
  readonly workspacePath?: string;
}

interface LaunchData {
  readonly api: typeof LAUNCH_API;
  readonly workspacePath: string | null;
}

/** Parse the argv shape emitted by a packaged Electron app or Forge development. */
export function parseLaunchRequest(
  argv: readonly string[],
  workingDirectory: string,
  packaged: boolean,
): LaunchRequest {
  const args = argv.slice(packaged ? 1 : 2);
  const positional: string[] = [];
  let explicitWorkspace: string | undefined;
  let afterSeparator = false;

  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === undefined) continue;
    if (!afterSeparator && argument === "--") {
      afterSeparator = true;
      continue;
    }
    if (!afterSeparator && argument === "--workspace") {
      const value = args[index + 1];
      if (!value) throw usageError("--workspace requires a path");
      explicitWorkspace = value;
      index += 1;
      continue;
    }
    // Electron and Squirrel can append switches. Exact user arguments are
    // carried separately through requestSingleInstanceLock additionalData.
    if (!afterSeparator && argument.startsWith("-")) continue;
    positional.push(argument);
  }

  if (explicitWorkspace && positional.length > 0) {
    throw usageError("pass the workspace either positionally or with --workspace");
  }
  if (positional.length > 1) throw usageError("only one workspace may be opened");

  const raw = explicitWorkspace ?? positional[0];
  if (raw === undefined) return {};
  return { workspacePath: normalizeWorkspacePath(raw, workingDirectory) };
}

/** Stable JSON-only payload for Electron's single-instance handoff. */
export function launchData(request: LaunchRequest): LaunchData {
  return {
    api: LAUNCH_API,
    workspacePath: request.workspacePath ?? null,
  };
}

/** Reject malformed second-instance data instead of falling back to reordered argv. */
export function parseLaunchData(value: unknown): LaunchRequest {
  if (!isRecord(value) || value.api !== LAUNCH_API) return {};
  if (value.workspacePath === null) return {};
  if (typeof value.workspacePath !== "string") return {};
  if (!isAbsolute(value.workspacePath) || !validPathText(value.workspacePath)) return {};
  return { workspacePath: value.workspacePath };
}

/** Serialize workspace changes so initial and second-instance opens cannot race. */
export class WorkspaceOpenQueue<Result> {
  private tail: Promise<void> = Promise.resolve();

  constructor(private readonly openRoot: (root: string) => Promise<Result>) {}

  open(root: string): Promise<Result> {
    const result = this.tail.then(() => this.openRoot(root));
    this.tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

function normalizeWorkspacePath(raw: string, workingDirectory: string): string {
  if (!validPathText(raw)) throw usageError("workspace path is empty or invalid");
  return resolve(workingDirectory, raw);
}

function validPathText(value: string): boolean {
  return (
    value.length > 0 &&
    [...value].length <= MAX_PATH_CHARACTERS &&
    !value.includes("\0") &&
    !value.includes("\r") &&
    !value.includes("\n")
  );
}

function usageError(detail: string): Error {
  return new Error(`Usage: motivo-studio [WORKSPACE] (${detail}).`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
