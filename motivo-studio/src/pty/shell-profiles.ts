import { existsSync } from "node:fs";
import { join } from "node:path";
import type { TerminalProfile } from "../shared/contracts";

export interface ResolvedShellProfile extends TerminalProfile {
  readonly executable?: string;
  readonly args: readonly string[];
}

export function resolveShellProfiles(
  platform: NodeJS.Platform = process.platform,
  environment: NodeJS.ProcessEnv = process.env,
): readonly ResolvedShellProfile[] {
  if (platform === "win32") {
    const candidates = [
      environment.ProgramFiles
        ? join(environment.ProgramFiles, "PowerShell", "7", "pwsh.exe")
        : undefined,
      environment.SystemRoot
        ? join(environment.SystemRoot, "System32", "WindowsPowerShell", "v1.0", "powershell.exe")
        : undefined,
    ].filter((candidate): candidate is string => candidate !== undefined);
    const powershell = candidates.find((candidate) => existsSync(candidate));
    const bashCandidates = [
      environment.ProgramFiles
        ? join(environment.ProgramFiles, "Git", "bin", "bash.exe")
        : undefined,
      environment["ProgramFiles(x86)"]
        ? join(environment["ProgramFiles(x86)"], "Git", "bin", "bash.exe")
        : undefined,
    ].filter((candidate): candidate is string => candidate !== undefined);
    const bash = bashCandidates.find((candidate) => existsSync(candidate));
    return [
      {
        id: "powershell",
        label: "PowerShell",
        available: powershell !== undefined,
        ...(powershell ? { executable: powershell } : {}),
        args: ["-NoLogo", "-NoProfile"],
      },
      {
        id: "bash",
        label: "Bash",
        available: bash !== undefined,
        ...(bash ? { executable: bash } : {}),
        args: ["--noprofile", "--norc"],
      },
    ];
  }
  const bash = "/bin/bash";
  return [
    { id: "powershell", label: "PowerShell", available: false, args: [] },
    {
      id: "bash",
      label: "Bash",
      available: existsSync(bash),
      ...(existsSync(bash) ? { executable: bash } : {}),
      args: ["--noprofile", "--norc"],
    },
  ];
}

export function terminalEnvironment(
  environment: NodeJS.ProcessEnv = process.env,
): Record<string, string> {
  const selected: Record<string, string> = { TERM: "xterm-256color", COLORTERM: "truecolor" };
  for (const key of [
    "SystemRoot",
    "WINDIR",
    "PATH",
    "PATHEXT",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "LANG",
  ]) {
    const value = environment[key];
    if (value !== undefined) selected[key] = value;
  }
  return selected;
}
