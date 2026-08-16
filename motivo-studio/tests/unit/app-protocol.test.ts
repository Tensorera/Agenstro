import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { resolveAppAsset } from "../../src/main/windows/app-assets";

describe("packaged renderer protocol", () => {
  it("maps only motivo app assets beneath the renderer root", () => {
    const root = resolve(import.meta.dirname, "renderer-root");
    expect(resolveAppAsset(root, "motivo://app/assets/index.js")).toBe(
      resolve(root, "assets/index.js"),
    );
    expect(resolveAppAsset(root, "motivo://other/assets/index.js")).toBeNull();
    expect(resolveAppAsset(root, "https://app/assets/index.js")).toBeNull();
    expect(resolveAppAsset(root, "motivo://app/%2e%2e%2fsecrets.txt")).toBeNull();
  });
});
