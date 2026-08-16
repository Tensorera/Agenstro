import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { loadNodePty } from "../../src/pty/node-pty-loader";
import { resolveShellProfiles, terminalEnvironment } from "../../src/pty/shell-profiles";

describe("locked node-pty prebuild", () => {
  it("spawns and reaps a real local shell without loading a user profile", async () => {
    const root = resolve(import.meta.dirname, "../..");
    const spawnPty = loadNodePty(resolve(root, "node_modules", "node-pty"));
    const profile = resolveShellProfiles().find((candidate) => candidate.available);
    expect(profile?.executable).toBeDefined();
    if (!profile?.executable) return;

    const terminal = spawnPty(profile.executable, [...profile.args], {
      name: "xterm-256color",
      cols: 80,
      rows: 24,
      cwd: root,
      env: terminalEnvironment(),
    });
    let output = "";
    terminal.onData((chunk) => {
      output = `${output}${chunk}`.slice(-65_536);
    });
    const exited = new Promise<void>((resolveExit, reject) => {
      const timer = setTimeout(() => {
        terminal.kill();
        reject(new Error("The PTY shell did not exit before its test deadline."));
      }, 10_000);
      terminal.onExit(() => {
        clearTimeout(timer);
        resolveExit();
      });
    });
    terminal.write(
      process.platform === "win32"
        ? "Write-Output 'MOTIVO_PTY_OK'; exit\r"
        : "printf 'MOTIVO_PTY_OK\\n'; exit\n",
    );
    await exited;
    expect(output).toContain("MOTIVO_PTY_OK");
  });
});
