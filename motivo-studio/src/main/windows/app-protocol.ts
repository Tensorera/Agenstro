import { pathToFileURL } from "node:url";
import { net, protocol } from "electron";
import { resolveAppAsset } from "./app-assets";

export function registerAppScheme(): void {
  protocol.registerSchemesAsPrivileged([
    {
      scheme: "motivo",
      privileges: {
        standard: true,
        secure: true,
        supportFetchAPI: true,
        stream: true,
      },
    },
  ]);
}

export function installAppProtocol(rendererRoot: string): void {
  protocol.handle("motivo", (request) => {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response(null, { status: 405 });
    }
    const asset = resolveAppAsset(rendererRoot, request.url);
    if (!asset) return new Response(null, { status: 404 });
    return net.fetch(pathToFileURL(asset).href, { method: request.method });
  });
}
