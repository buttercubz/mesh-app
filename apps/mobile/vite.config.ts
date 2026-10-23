// Vite 8 config (WP-18, DESIGN §11.3): React 19 with the React Compiler (no StrictMode),
// Tailwind v4, `@/` → src. Workspace packages are consumed from their dist/ builds.
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { defineConfig } from "vite";

/**
 * The shipped CSP lives in index.html (`default-src 'self'`, contract-tested). The dev
 * server needs an inline React Refresh preamble, injected <style> tags and the HMR
 * WebSocket, so `vite dev` (and only it) serves a relaxed copy of the policy.
 */
function devCsp(): Plugin {
  return {
    name: "meshapp:dev-csp",
    apply: "serve",
    transformIndexHtml(html) {
      return html.replace(
        /(<meta\s+http-equiv="Content-Security-Policy"\s+content=")([^"]*)(")/,
        (_m, open: string, policy: string, close: string) =>
          open +
          policy
            .replace("script-src 'self'", "script-src 'self' 'unsafe-inline'")
            .replace("style-src 'self'", "style-src 'self' 'unsafe-inline'")
            .replace("connect-src 'self'", "connect-src 'self' ws: wss:") +
          close,
      );
    },
  };
}

export default defineConfig({
  plugins: [
    react({
      babel: {
        plugins: [["babel-plugin-react-compiler", {}]],
      },
    }),
    tailwindcss(),
    devCsp(),
  ],
  resolve: {
    // One React and one Capacitor core for the app and the workspace packages (ui-kit's
    // own devDependency React must never be bundled as a second copy).
    dedupe: ["react", "react-dom", "@capacitor/core"],
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "es2022",
    // Capacitor serves dist/ from the device: no source maps in the shipped bundle.
    sourcemap: false,
    assetsInlineLimit: 0,
  },
});
