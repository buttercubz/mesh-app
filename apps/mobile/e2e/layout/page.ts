// Page-side plumbing of the layout audit: device emulation (viewport, touch, locale,
// safe-area insets through CDP), the textZoom emulation, the dev-only store hook
// (src/app/layoutHarness.ts, `?simDebug=1` on `vite dev`) and settling.
import type { Browser, BrowserContext, Page } from "@playwright/test";
import type { Snapshot, TalkDeniedReason } from "@meshapp/engine-api";

import type { LayoutDevice, Locale } from "./devices";

/** This phone plus two simulated riders, all in range, nobody talking (fast, deterministic). */
export const SIM_URL = "/?sim=pair&nodes=3&botsTalk=0&seed=7&simDebug=1";

/** The rider of this phone (long enough to stress the profile header). */
export const MY_NAME = "Maximiliano R.";

export interface InsetEmulation {
  /** "cdp": Emulation.setSafeAreaInsetsOverride; "none": not available (see limitations). */
  method: "cdp" | "none";
  /** What env(safe-area-inset-*) reports after the override (probe element). */
  envTop: number | null;
  envBottom: number | null;
  error?: string;
}

export async function openDevice(
  browser: Browser,
  baseURL: string,
  device: LayoutDevice,
  locale: Locale,
): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser.newContext({
    baseURL,
    viewport: { width: device.width, height: device.height },
    screen: { width: device.width, height: device.height },
    deviceScaleFactor: device.deviceScaleFactor,
    isMobile: true,
    hasTouch: true,
    userAgent: device.userAgent,
    locale: locale === "es" ? "es-ES" : "en-US",
    colorScheme: "dark",
    // The app honours reduced motion (motion-reduce:animate-none): no entry animations
    // between a state change and its measurement.
    reducedMotion: "reduce",
  });
  const page = await context.newPage();
  await page.addInitScript(
    ({ language, name }) => {
      if (localStorage.getItem("meshapp.settings") !== null) return;
      const state = { language, vehicle: "moto", displayName: name, onboardingDone: true, theme: "dark" };
      localStorage.setItem("meshapp.settings", JSON.stringify({ state, version: 1 }));
    },
    { language: locale, name: MY_NAME },
  );
  return { context, page };
}

/** Applies the device insets with CDP and reads them back through env(). */
export async function emulateInsets(page: Page, device: LayoutDevice): Promise<InsetEmulation> {
  let error: string | undefined;
  try {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Emulation.setSafeAreaInsetsOverride" as never, { insets: { ...device.insets } } as never);
  } catch (e) {
    error = (e as Error).message;
  }
  const read = await readEnvInsets(page);
  const ok = read.top === device.insets.top && read.bottom === device.insets.bottom;
  return { method: ok ? "cdp" : "none", envTop: read.top, envBottom: read.bottom, ...(error !== undefined ? { error } : {}) };
}

export async function readEnvInsets(page: Page): Promise<{ top: number; bottom: number }> {
  return page.evaluate(() => {
    const probe = document.createElement("div");
    probe.style.cssText =
      "position:fixed;left:0;top:0;width:1px;visibility:hidden;padding-top:env(safe-area-inset-top,0px);padding-bottom:env(safe-area-inset-bottom,0px)";
    document.body.appendChild(probe);
    const s = getComputedStyle(probe);
    const out = { top: parseFloat(s.paddingTop), bottom: parseFloat(s.paddingBottom) };
    probe.remove();
    return out;
  });
}

/**
 * Android WebView textZoom: the app sizes text in rem, so scaling the root font size
 * scales every rem font. px font sizes escape it (listed by findPxFonts); rem spacing
 * scales too, which is slightly conservative compared with a text-only zoom.
 */
export async function setTextZoom(page: Page, zoom: number): Promise<void> {
  await page.evaluate((z) => {
    let style = document.getElementById("layout-text-zoom");
    if (style === null) {
      style = document.createElement("style");
      style.id = "layout-text-zoom";
      document.head.appendChild(style);
    }
    style.textContent = z === 100 ? "" : `html { font-size: ${(16 * z) / 100}px !important; }`;
  }, zoom);
}

/** Two animation frames plus pending fonts and images: the layout is final. */
export async function settle(page: Page, extraMs = 60): Promise<void> {
  await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all(
      [...document.images].filter((i) => !i.complete).map((i) => new Promise((r) => i.addEventListener("load", r, { once: true }))),
    );
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
  if (extraMs > 0) await page.waitForTimeout(extraMs);
}

/** Typed calls into the dev-only hook (`globalThis.__layoutHarness`). */
export class Hook {
  readonly page: Page;

  constructor(page: Page) {
    this.page = page;
  }

  async ready(): Promise<void> {
    await this.page.waitForFunction(() => (globalThis as { __layoutHarness?: unknown }).__layoutHarness !== undefined, null, {
      timeout: 30_000,
    });
  }

  private call<T>(method: string, ...args: unknown[]): Promise<T> {
    return this.page.evaluate(
      ({ method, args }) => {
        const h = (globalThis as unknown as { __layoutHarness: Record<string, (...a: unknown[]) => unknown> }).__layoutHarness;
        const fn = h[method];
        if (fn === undefined) throw new Error(`layout hook has no ${method}`);
        return fn(...args) as T;
      },
      { method, args },
    );
  }

  liveSnapshot(): Promise<Snapshot | null> {
    return this.call("liveSnapshot");
  }
  freeze(snapshot: Snapshot, extra: { status?: "idle" | "starting" | "running"; compat?: "unknown" | "ok" | "mismatch" } = {}) {
    return this.call<void>("freeze", snapshot, extra);
  }
  talkDenied(reason: TalkDeniedReason | null, agoMs = 0) {
    return this.call<void>("talkDenied", reason, agoMs);
  }
  unfreeze() {
    return this.call<void>("unfreeze");
  }
  go(id: string, params: Record<string, string | number | boolean | null> = {}, then: Array<[string, Record<string, string | number | boolean | null>?]> = []) {
    return this.call<void>("go", id, params, then);
  }
  stack(): Promise<string[]> {
    return this.call("stack");
  }
  settings(patch: Record<string, unknown>) {
    return this.call<void>("settings", patch);
  }
  groups(list: Array<{ id: string; name: string; role: "leader" | "member"; lastUsed: number }>) {
    return this.call<void>("groups", list);
  }
  toast(input: { title: string; description?: string; tone?: "neutral" | "success" | "warn" | "danger"; durationMs?: number; key?: string }) {
    return this.call<void>("toast", input);
  }
  clearToasts() {
    return this.call<void>("clearToasts");
  }
  platform(platform: "ios" | "android" | "web") {
    return this.call<void>("platform", platform);
  }
  aware(status: { supported: boolean; enabled: boolean } | null) {
    return this.call<void>("aware", status);
  }
  t(namespace: string, key: string, params: Record<string, unknown> = {}): Promise<string> {
    return this.call("t", namespace, key, params);
  }
}
