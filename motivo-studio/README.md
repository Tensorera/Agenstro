# Motivo Studio

> **0.3 refactor status: frozen.** Motivo is not part of the Haskell DSL
> cutover release gate. The Electron code below documents the 0.2 alpha. If
> revived, Motivo will be reduced to a projection of Tactus plugin discovery
> and smoke results; it will not own a second workflow runtime or daemon.

`motivo-studio` is the `0.2.0` alpha Electron desktop client. The renderer is a
projection only: Electron main owns daemon and PTY connections, preload exposes
a named Zod-validated bridge, and the renderer has no Node integration or
daemon token access.

## Minimal Success Path

With Node.js 22.12 or newer and the checked-in lockfile:

```powershell
npm ci
npm run generate
npm test
npm run build
```

The build creates a current-platform Electron package, not a signed installer.
The package does not currently include daemon binaries, so launch falls back to
an explicit degraded state. Do not interpret packaging as daemon E2E evidence.

## References

- [Window security policy](src/main/windows/security.ts)
- [Daemon bootstrap contract](src/main/daemon/bootstrap.ts)
- [Narrow preload bridge](src/preload/bridge.ts)
- [Forge packaging configuration](forge.config.ts)
- [Runtime boundary](../docs/explanation/runtime-boundaries.md)
