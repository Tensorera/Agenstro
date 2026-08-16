import { isAbsolute, resolve } from "node:path";
import { describe, expect, it, vi } from "vitest";
import {
  launchData,
  parseLaunchData,
  parseLaunchRequest,
  WorkspaceOpenQueue,
} from "../../src/main/launch";

describe("Motivo launch request parsing", () => {
  it("parses packaged and Forge development argv shapes", () => {
    const workingDirectory = resolve("workspace-parent");
    const workspace = resolve(workingDirectory, "project");

    expect(
      parseLaunchRequest([resolve("motivo-studio.exe"), workspace], workingDirectory, true),
    ).toEqual({ workspacePath: workspace });
    expect(
      parseLaunchRequest(
        [resolve("electron.exe"), resolve("motivo-studio"), workspace],
        workingDirectory,
        false,
      ),
    ).toEqual({ workspacePath: workspace });
  });

  it("resolves a relative workspace against the launching process directory", () => {
    const workingDirectory = resolve("parent", "launch-directory");

    const request = parseLaunchRequest(
      [resolve("motivo-studio.exe"), "../target"],
      workingDirectory,
      true,
    );

    expect(request).toEqual({ workspacePath: resolve(workingDirectory, "../target") });
    expect(request.workspacePath && isAbsolute(request.workspacePath)).toBe(true);
  });

  it("preserves spaces and Chinese characters in one positional path", () => {
    const workingDirectory = resolve("launch-directory");
    const relativeWorkspace = "项目 工作区";

    expect(
      parseLaunchRequest([resolve("motivo-studio.exe"), relativeWorkspace], workingDirectory, true),
    ).toEqual({ workspacePath: resolve(workingDirectory, relativeWorkspace) });
  });

  it("accepts an explicit --workspace value", () => {
    const workingDirectory = resolve("launch-directory");

    expect(
      parseLaunchRequest(
        [resolve("motivo-studio.exe"), "--workspace", "项目 工作区"],
        workingDirectory,
        true,
      ),
    ).toEqual({ workspacePath: resolve(workingDirectory, "项目 工作区") });
  });

  it("treats a switch-looking path after -- as positional", () => {
    const workingDirectory = resolve("launch-directory");

    expect(
      parseLaunchRequest(
        [resolve("motivo-studio.exe"), "--", "-workspace"],
        workingDirectory,
        true,
      ),
    ).toEqual({ workspacePath: resolve(workingDirectory, "-workspace") });
  });

  it("returns an empty request when no workspace was supplied", () => {
    expect(parseLaunchRequest([resolve("motivo-studio.exe")], resolve("cwd"), true)).toEqual({});
    expect(
      parseLaunchRequest(
        [resolve("electron.exe"), resolve("motivo-studio")],
        resolve("cwd"),
        false,
      ),
    ).toEqual({});
  });

  it("rejects multiple workspace paths and mixed explicit and positional forms", () => {
    const executable = resolve("motivo-studio.exe");
    const workingDirectory = resolve("cwd");

    expect(() =>
      parseLaunchRequest([executable, "first", "second"], workingDirectory, true),
    ).toThrow(/only one workspace/i);
    expect(() =>
      parseLaunchRequest([executable, "--workspace", "first", "second"], workingDirectory, true),
    ).toThrow(/either positionally or with --workspace/i);
  });
});

describe("Motivo single-instance launch data", () => {
  it("round-trips an absolute workspace and encodes an absent workspace as null", () => {
    const workspacePath = resolve("项目 工作区");

    const data = launchData({ workspacePath });
    expect(data).toEqual({ api: "motivo.launch/v1", workspacePath });
    expect(parseLaunchData(data)).toEqual({ workspacePath });
    expect(launchData({})).toEqual({ api: "motivo.launch/v1", workspacePath: null });
    expect(parseLaunchData(launchData({}))).toEqual({});
  });

  it.each([
    null,
    [],
    {},
    { api: "motivo.launch/v0", workspacePath: resolve("workspace") },
    { api: "motivo.launch/v1" },
    { api: "motivo.launch/v1", workspacePath: 42 },
    { api: "motivo.launch/v1", workspacePath: "relative-workspace" },
    { api: "motivo.launch/v1", workspacePath: `${resolve("workspace")}\0suffix` },
    { api: "motivo.launch/v1", workspacePath: `${resolve("workspace")}\nsecond` },
  ])("rejects malformed additionalData without trusting fallback argv: %j", (value) => {
    expect(parseLaunchData(value)).toEqual({});
  });
});

describe("WorkspaceOpenQueue", () => {
  it("opens workspaces serially", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const openRoot = vi
      .fn<(root: string) => Promise<string>>()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const queue = new WorkspaceOpenQueue(openRoot);

    const firstOpen = queue.open("first");
    const secondOpen = queue.open("second");
    await Promise.resolve();

    expect(openRoot).toHaveBeenCalledTimes(1);
    expect(openRoot).toHaveBeenNthCalledWith(1, "first");

    first.resolve("first-view");
    await expect(firstOpen).resolves.toBe("first-view");
    await Promise.resolve();
    expect(openRoot).toHaveBeenCalledTimes(2);
    expect(openRoot).toHaveBeenNthCalledWith(2, "second");

    second.resolve("second-view");
    await expect(secondOpen).resolves.toBe("second-view");
  });

  it("continues with the next workspace after an earlier open fails", async () => {
    const first = deferred<string>();
    const openRoot = vi
      .fn<(root: string) => Promise<string>>()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce("second-view");
    const queue = new WorkspaceOpenQueue(openRoot);

    const firstOpen = queue.open("broken");
    const secondOpen = queue.open("healthy");
    await Promise.resolve();
    expect(openRoot).toHaveBeenCalledOnce();

    first.reject(new Error("inspect failed"));
    await expect(firstOpen).rejects.toThrow("inspect failed");
    await expect(secondOpen).resolves.toBe("second-view");
    expect(openRoot).toHaveBeenNthCalledWith(2, "healthy");
  });
});

function deferred<Value>(): {
  readonly promise: Promise<Value>;
  readonly resolve: (value: Value) => void;
  readonly reject: (reason: unknown) => void;
} {
  let resolvePromise: ((value: Value) => void) | undefined;
  let rejectPromise: ((reason: unknown) => void) | undefined;
  const promise = new Promise<Value>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  return {
    promise,
    resolve: (value) => resolvePromise?.(value),
    reject: (reason) => rejectPromise?.(reason),
  };
}
