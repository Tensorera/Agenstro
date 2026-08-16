import { globSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");

describe("renderer authority boundary", () => {
  it("contains no host, transport, subprocess, or direct network authority", () => {
    const files = globSync("src/renderer/**/*.{ts,tsx}", { cwd: root });
    expect(files.length).toBeGreaterThan(0);
    for (const file of files) {
      const source = readFileSync(resolve(root, file), "utf8");
      expect(source, file).not.toMatch(/from\s+["'](?:node:|electron|@grpc\/)/);
      expect(source, file).not.toMatch(/from\s+["'][^"']*(?:main|preload|generated)\//);
      expect(source, file).not.toMatch(/\b(?:fetch|XMLHttpRequest|WebSocket|ipcRenderer|spawn)\b/);
      expect(source, file).not.toContain("dangerouslySetInnerHTML");
    }
  });

  it("never asks the renderer for a workspace root", () => {
    for (const file of globSync("src/renderer/**/*.{ts,tsx}", { cwd: root })) {
      const source = readFileSync(resolve(root, file), "utf8");
      expect(source, file).not.toMatch(/workspaceRoot|absolutePath|--root/);
    }
  });
});
