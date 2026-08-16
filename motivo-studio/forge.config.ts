import type { ForgeConfig } from "@electron-forge/shared-types";
import { MakerSquirrel } from "@electron-forge/maker-squirrel";
import { MakerZIP } from "@electron-forge/maker-zip";
import { AutoUnpackNativesPlugin } from "@electron-forge/plugin-auto-unpack-natives";
import { FusesPlugin } from "@electron-forge/plugin-fuses";
import { VitePlugin } from "@electron-forge/plugin-vite";
import { FuseV1Options, FuseVersion } from "@electron/fuses";

const config: ForgeConfig = {
  packagerConfig: {
    asar: true,
    executableName: "motivo-studio",
    name: "Motivo Studio",
    extraResource: ["node_modules/node-pty"],
  },
  // node-pty 1.1 ships target-platform N-API prebuilds. Rebuilding would replace
  // those stable artifacts and unnecessarily require a local MSVC toolchain.
  rebuildConfig: { onlyModules: [] },
  makers: [new MakerSquirrel({}), new MakerZIP({}, ["win32", "linux"])],
  plugins: [
    new AutoUnpackNativesPlugin({}),
    new VitePlugin({
      build: [
        { entry: "src/main.ts", config: "vite.main.config.ts" },
        { entry: "src/preload.ts", config: "vite.preload.config.ts" },
        { entry: "src/pty-host.ts", config: "vite.pty.config.ts" },
      ],
      renderer: [{ name: "main_window", config: "vite.renderer.config.ts" }],
      concurrent: 2,
    }),
    new FusesPlugin({
      version: FuseVersion.V1,
      // Forge 7's supported @electron/fuses v1 surface predates Electron 43's
      // WasmTrapHandlers bit. Keep that performance fuse at its upstream default.
      strictlyRequireAllFuses: false,
      [FuseV1Options.RunAsNode]: false,
      [FuseV1Options.EnableCookieEncryption]: true,
      [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
      [FuseV1Options.EnableNodeCliInspectArguments]: false,
      [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: true,
      [FuseV1Options.OnlyLoadAppFromAsar]: true,
      [FuseV1Options.LoadBrowserProcessSpecificV8Snapshot]: false,
      [FuseV1Options.GrantFileProtocolExtraPrivileges]: false,
    }),
  ],
};

export default config;
