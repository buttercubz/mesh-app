// Shell e2e (WP-18): the app boots under its CSP in a phone viewport, talks to the web
// implementation of MeshIntercom through the engine client (getInfo → API check),
// localises from the device language, and logs no errors. Only shell-owned markers are
// asserted, so the spec stays valid once features register their screens (WP-21/22).
import type { ConsoleMessage, Page } from "@playwright/test";
import { expect, test } from "@playwright/test";

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m: ConsoleMessage) => {
    if (m.type() === "error") errors.push(`console: ${m.text()}`);
  });
  return errors;
}

test.describe("app shell (English device)", () => {
  test.use({ locale: "en-US" });

  test("boots, checks the engine API and renders the start screen", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await expect(page).toHaveTitle("convLink Voice");
    await expect(page.locator("html")).toHaveAttribute("lang", "en");
    const shell = page.locator("[data-screen]");
    await expect(shell).toHaveAttribute("data-screen", "home");
    await expect(shell).toHaveAttribute("data-engine-platform", "web");
    await expect(shell).toHaveAttribute("data-api-compat", "ok");
    await expect(page.getByRole("main", { name: "Main content" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Notifications" })).toBeAttached();
    await expect(page.locator("html")).toHaveClass(/dark/);
    expect(errors).toEqual([]);
  });

  test("enforces the CSP: no eval, no network to other origins", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("[data-screen]")).toHaveAttribute("data-engine-platform", "web");
    const csp = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute("content");
    expect(csp).toContain("default-src 'self'");
    const result = await page.evaluate(async () => {
      const violations: string[] = [];
      document.addEventListener("securitypolicyviolation", (e) => violations.push(e.effectiveDirective));
      const w = window as unknown as Record<string, unknown>;
      // A string timer is eval: it runs in the page (not through the test driver), so the
      // CSP decides. Without 'unsafe-eval' it must never execute.
      (setTimeout as unknown as (code: string, ms: number) => void)("window.__evalRan = true", 0);
      let fetchOutcome = "loaded";
      try {
        await fetch("https://example.com/");
      } catch (e) {
        fetchOutcome = (e as Error).name;
      }
      const img = new Image();
      img.src = "https://example.com/pixel.png";
      await new Promise((r) => setTimeout(r, 100));
      return { evalRan: w["__evalRan"] === true, fetchOutcome, violations };
    });
    expect(result.evalRan).toBe(false);
    expect(result.fetchOutcome).toBe("TypeError");
    expect(result.violations).toEqual(expect.arrayContaining(["script-src", "img-src"]));
    expect(result.violations).toEqual(expect.arrayContaining(["connect-src"]));
  });

  test("keeps the layout inside a phone viewport (no horizontal scroll)", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("[data-screen]")).toBeVisible();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test.describe("shipped bundle (vite preview of dist/)", () => {
  test.use({ locale: "en-US" });

  test("boots under the strict CSP with no violation and no error", async ({ page }) => {
    const errors = collectErrors(page);
    const violations: string[] = [];
    await page.exposeFunction("__reportViolation", (d: string) => violations.push(d));
    await page.addInitScript(() => {
      document.addEventListener("securitypolicyviolation", (e) => {
        (window as unknown as { __reportViolation(d: string): void }).__reportViolation(
          `${e.effectiveDirective} ${e.blockedURI}`,
        );
      });
    });
    await page.goto("http://127.0.0.1:4173/");
    const csp = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute("content");
    expect(csp).not.toContain("unsafe-inline");
    await expect(page.locator("[data-screen]")).toHaveAttribute("data-api-compat", "ok");
    await expect(page.getByRole("main", { name: "Main content" })).toBeVisible();
    expect(violations).toEqual([]);
    expect(errors).toEqual([]);
  });
});

test.describe("app shell (Spanish device)", () => {
  test.use({ locale: "es-ES" });

  test("starts in Spanish", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("lang", "es");
    await expect(page.getByRole("main", { name: "Contenido principal" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Notificaciones" })).toBeAttached();
    expect(errors).toEqual([]);
  });
});
