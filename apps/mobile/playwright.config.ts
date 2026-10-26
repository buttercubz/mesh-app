// Playwright config (WP-18, DESIGN §13.1): e2e against `vite dev` with the web
// implementation of MeshIntercom (Phase 0: the fake Lab peer; Phase 1: the WASM sim).
// Browsers are installed by scripts/bootstrap.sh (`playwright install chromium`).
import { defineConfig, devices } from "@playwright/test";

const PORT = 5173;
/** `vite preview` of the production build (dist/), for checks of the shipped CSP. */
export const PREVIEW_PORT = 4173;

export default defineConfig({
  testDir: "./e2e",
  // The layout audit has its own config (playwright.layout.config.ts, port 5174).
  testIgnore: ["**/layout.spec.ts"],
  fullyParallel: false,
  forbidOnly: !!process.env["CI"],
  retries: process.env["CI"] ? 1 : 0,
  reporter: [["list"]],
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: "retain-on-failure",
    // Phone-sized viewport: the app is designed for one-handed use.
    ...devices["Pixel 7"],
  },
  projects: [{ name: "chromium", use: { browserName: "chromium" } }],
  webServer: [
    {
      command: `pnpm exec vite --host 127.0.0.1 --port ${PORT} --strictPort`,
      url: `http://127.0.0.1:${PORT}`,
      reuseExistingServer: !process.env["CI"],
      timeout: 120_000,
    },
    {
      // Needs a prior `pnpm build` (every verify builds first).
      command: `pnpm exec vite preview --host 127.0.0.1 --port ${PREVIEW_PORT} --strictPort`,
      url: `http://127.0.0.1:${PREVIEW_PORT}`,
      reuseExistingServer: !process.env["CI"],
      timeout: 60_000,
    },
  ],
});
