import { globSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");

describe("renderer authority boundary", () => {
  it("contains no Node, Electron, gRPC, generated-client, or pywebview imports", () => {
    const files = globSync("src/renderer/**/*.{ts,tsx}", { cwd: root });
    expect(files.length).toBeGreaterThan(0);
    for (const file of files) {
      const source = readFileSync(resolve(root, file), "utf8");
      expect(source, file).not.toMatch(/from\s+["'](?:node:|electron|@grpc\/)/);
      expect(source, file).not.toMatch(/from\s+["'][^"']*(?:main|preload|generated)\//);
      expect(source, file).not.toContain("pywebview");
      expect(source, file).not.toContain("dangerouslySetInnerHTML");
    }
  });

  it("keeps generated runtime code free of explicit any", () => {
    const files = globSync("src/generated/**/*.ts", { cwd: root });
    for (const file of files) {
      const source = readFileSync(resolve(root, file), "utf8")
        .replaceAll(/\/\/.*$/gm, "")
        .replaceAll(/\/\*[\s\S]*?\*\//g, "");
      expect(source, file).not.toMatch(/\bany\b/);
    }
  });

  it("keeps scheduler data on the typed Motivo bridge", () => {
    const source = readFileSync(
      resolve(root, "src/renderer/components/SchedulerPanel.tsx"),
      "utf8",
    );
    expect(source).toContain("window.motivo.schedules.listPage");
    expect(source).not.toMatch(/\b(?:fetch|XMLHttpRequest|WebSocket|ipcRenderer)\b/);
    expect(source).not.toContain("pywebview");
  });
});
