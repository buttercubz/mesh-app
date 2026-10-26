// Layout audit config (e2e/layout.spec.ts): its own `vite dev` on LAYOUT_PORT (default
// 5174, strict, never reused) so several audits can run next to the e2e server on 5173.
// The dev server is required: the audit drives the dev-only hook (src/app/layoutHarness.ts,
// `?simDebug=1`) that production bundles do not contain.
//
//   LAYOUT_AUDIT=1 LAYOUT_OUT=<dir> pnpm --filter @meshapp/mobile exec playwright test -c playwright.layout.config.ts
//   pnpm --filter @meshapp/mobile exec playwright test -c playwright.layout.config.ts   (CI subset, asserts)
import { defineConfig } from "@playwright/test";

const PORT = Number(process.env["LAYOUT_PORT"] ?? 5174);
const AUDIT = process.env["LAYOUT_AUDIT"] === "1";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "layout.spec.ts",
  fullyParallel: true,
  workers: Number(process.env["LAYOUT_WORKERS"] ?? 4),
  forbidOnly: !!process.env["CI"],
  retries: 0,
  reporter: [["list"]],
  timeout: AUDIT ? 30 * 60_000 : 180_000,
  globalSetup: "./e2e/layout/global-setup.ts",
  globalTeardown: "./e2e/layout/global-teardown.ts",
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    browserName: "chromium",
    trace: "off",
  },
  projects: [{ name: "layout", use: { browserName: "chromium" } }],
  webServer: {
    command: `pnpm exec vite --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
