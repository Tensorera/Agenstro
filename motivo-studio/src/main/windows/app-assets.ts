import { resolve, sep } from "node:path";

export function resolveAppAsset(rendererRoot: string, requestUrl: string): string | null {
  try {
    const url = new URL(requestUrl);
    if (url.protocol !== "motivo:" || url.hostname !== "app") return null;
    const relative = decodeURIComponent(url.pathname).replace(/^\/+/, "");
    if (!relative || relative.includes("\0")) return null;
    const root = resolve(rendererRoot);
    const candidate = resolve(root, relative);
    if (candidate !== root && !candidate.startsWith(`${root}${sep}`)) return null;
    return candidate;
  } catch {
    return null;
  }
}
