import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  base: "./",
  plugins: [
    react(),
    {
      name: "motivo-development-style-csp",
      apply: "serve",
      transformIndexHtml(html) {
        // Vite HMR injects CSS through local style elements. Production keeps
        // the stricter source CSP and emits a same-origin stylesheet asset.
        return html.replace("style-src 'self'", "style-src 'self' 'unsafe-inline'");
      },
    },
  ],
  build: {
    sourcemap: false,
    modulePreload: { polyfill: false },
  },
});
