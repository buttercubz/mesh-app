// Vitest config (WP-18, DESIGN §13.1). Default environment is node; component tests opt
// into jsdom with a `// @vitest-environment jsdom` docblock. Static contract tests live
// in tests/*.test.mjs; feature tests may sit next to their code (src/**/*.test.ts[x]).
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    // One React and one Capacitor core for the app and the workspace packages (ui-kit's
    // own devDependency React must never be bundled as a second copy).
    dedupe: ["react", "react-dom", "@capacitor/core"],
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.{ts,tsx,mjs}", "src/**/*.test.{ts,tsx}"],
    exclude: ["**/node_modules/**", "**/dist/**", "e2e/**", "ios/**", "android/**"],
    setupFiles: ["tests/setup.ts"],
    restoreMocks: true,
    // Node ≥ 25 ships an experimental global localStorage that warns on access; the
    // WebView's (or jsdom's) storage is what the app uses.
    execArgv: ["--no-experimental-webstorage"],
  },
});
