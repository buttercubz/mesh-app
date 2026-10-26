// Layout audit (run with playwright.layout.config.ts, never with the e2e config): every
// screen and important state on emulated Android and iOS phones (viewport, touch, locale,
// safe-area insets through CDP, Android textZoom), measured in the page (overflow, scroll,
// hidden or covered controls, truncation, bad wraps, safe-area zones, touch targets, PTT
// size) with a screenshot per (screen, state, device, zoom, locale).
//
//   LAYOUT_AUDIT=1  full matrix, report only (never fails on layout problems)
//   default         CI regression subset (a-small@130 es, i-se@100 es, i-std@100 en,
//                   a-tablet@100 es; key screens + worst ride stacking) with assertions
//   LAYOUT_OUT      output directory (report.json, summary.md, shots/)
//   LAYOUT_DEVICES  comma list of device ids to run (both modes)
//   LAYOUT_STATES   comma list of "screen" or "screen/state" filters (both modes)
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "@playwright/test";

import type { LayoutDevice, Locale } from "./layout/devices";
import { CI_SUBSET, DEVICES, EXTREME_ZOOMS, LOCALES, deviceById } from "./layout/devices";
import type { MeasureResult } from "./layout/measure";
import { findPxFonts, measureLayout } from "./layout/measure";
import { Hook, SIM_URL, emulateInsets, openDevice, setTextZoom, settle } from "./layout/page";
import type { CaptureRecord, ReportPart, ReportProblem } from "./layout/report";
import { outDir, writePart } from "./layout/report";
import type { LayoutState, StateContext } from "./layout/states";
import { STATES, afterZoom } from "./layout/states";

const AUDIT = process.env["LAYOUT_AUDIT"] === "1";
const DEVICE_FILTER = (process.env["LAYOUT_DEVICES"] ?? "").split(",").filter((s) => s !== "");
const STATE_FILTER = (process.env["LAYOUT_STATES"] ?? "").split(",").filter((s) => s !== "");

interface Run {
  readonly device: LayoutDevice;
  readonly locale: Locale;
  readonly zooms: readonly number[];
}

const RUNS: Run[] = (
  AUDIT
    ? DEVICES.flatMap((device) => LOCALES.map((locale) => ({ device, locale, zooms: device.zooms })))
    : CI_SUBSET.map((c) => ({ device: deviceById(c.device), locale: c.locale, zooms: [c.zoom] }))
).filter((r) => DEVICE_FILTER.length === 0 || DEVICE_FILTER.includes(r.device.id));

function wanted(s: LayoutState, run: Run): boolean {
  if (s.platform !== undefined && s.platform !== run.device.platform) return false;
  if (!AUDIT && s.ci !== true) return false;
  if (STATE_FILTER.length === 0) return true;
  return STATE_FILTER.some((f) => f === s.screen || f === `${s.screen}/${s.state}`);
}

/** The CI subset fails on these (DESIGN: ride never scrolls, nothing under the system bars). */
function ciFailure(p: ReportProblem, s: LayoutState): boolean {
  if (p.kind === "harnessError") return p.severity === "error";
  if (p.kind === "hOverflow" || p.kind === "offscreenX" || p.kind === "underSafeArea" || p.kind === "hiddenPinned") return true;
  if (s.ride !== true) return false;
  if (p.kind === "pageVScroll" || p.kind === "pttSmall") return true;
  if (p.kind === "bodyScroll") return p.severity === "error";
  return p.kind === "hiddenKey" && /data-banner|ride-banners|mk-ptt|mute-toggle|radiogroup/.test(p.selector);
}

const shotPath = (run: Run, zoom: number, s: LayoutState) =>
  join("shots", run.device.id, run.locale, `z${zoom}`, `${s.screen}--${s.state}.jpg`);

test.describe.configure({ mode: "parallel" });

for (const run of RUNS) {
  const title = AUDIT ? `${run.device.id} ${run.locale}` : `${run.device.id} ${run.locale} @${run.zooms.join("/")}%`;

  test(title, async ({ browser, baseURL }) => {
    test.setTimeout(AUDIT ? 30 * 60_000 : 170_000);
    const started = Date.now();
    const out = outDir();
    const { context, page } = await openDevice(browser, baseURL ?? "", run.device, run.locale);
    const pageErrors: string[] = [];
    page.on("pageerror", (e) => pageErrors.push(e.message));

    const part: ReportPart = {
      device: run.device.id,
      platform: run.device.platform,
      locale: run.locale,
      mode: AUDIT ? "audit" : "ci",
      insets: { method: "none", envTop: null, envBottom: null },
      startedAt: new Date(started).toISOString(),
      durationMs: 0,
      captures: [],
      problems: [],
      pxFonts: [],
    };
    const ciFailures: string[] = [];

    try {
      await page.goto(SIM_URL);
      part.insets = await emulateInsets(page, run.device);
      const root = page.locator("[data-screen]");
      await expect(root).toHaveAttribute("data-screen", "home", { timeout: 30_000 });
      await expect(root).toHaveAttribute("data-engine-kind", "full", { timeout: 30_000 });
      const hook = new Hook(page);
      await hook.ready();
      await hook.platform(run.device.platform);

      const ctx: StateContext = { page, hook, device: run.device, locale: run.locale, base: null, groupId: null };
      const states = STATES.filter((s) => wanted(s, run));

      for (const s of states) {
        const base = { screen: s.screen, state: s.state, device: run.device.id, platform: run.device.platform, locale: run.locale };
        try {
          await setTextZoom(page, 100);
          await hook.clearToasts();
          await s.setup(ctx);
          await settle(page);
        } catch (e) {
          const message = (e as Error).message.split("\n")[0] ?? String(e);
          const shot = shotPath(run, 100, s).replace(/\.jpg$/, "--setup-failed.jpg");
          await saveShot(page, join(out, shot));
          part.problems.push({
            ...base,
            zoom: 100,
            kind: "harnessError",
            severity: "error",
            selector: "harness",
            testid: null,
            text: "",
            detail: `setup failed: ${message}`,
            magnitude: 0,
            screenshot: shot,
          });
          if (!AUDIT) ciFailures.push(`${s.screen}/${s.state}: setup failed: ${message}`);
          continue;
        }

        if (run.device.platform === "android" && AUDIT) {
          for (const f of await page.evaluate(findPxFonts)) part.pxFonts.push({ screen: s.screen, state: s.state, ...f });
        }

        for (const zoom of run.zooms) {
          const shot = shotPath(run, zoom, s);
          let result: MeasureResult;
          try {
            await setTextZoom(page, zoom);
            await afterZoom(ctx, s);
            result = await page.evaluate(measureLayout, {
              insetTop: run.device.insets.top,
              insetBottom: run.device.insets.bottom,
              ride: s.ride === true,
              extreme: EXTREME_ZOOMS.has(zoom),
            });
            await saveShot(page, join(out, shot));
          } catch (e) {
            part.problems.push({
              ...base,
              zoom,
              kind: "harnessError",
              severity: "error",
              selector: "harness",
              testid: null,
              text: "",
              detail: `measure failed: ${(e as Error).message.split("\n")[0] ?? ""}`,
              magnitude: 0,
              screenshot: shot,
            });
            continue;
          }
          const problems: ReportProblem[] = result.problems.map((p) => ({ ...base, zoom, ...p, screenshot: shot }));
          part.problems.push(...problems);
          const capture: CaptureRecord = { ...base, zoom, screenshot: shot, metrics: { ...result.metrics }, problemCount: problems.length };
          part.captures.push(capture);
          if (!AUDIT) {
            for (const p of problems.filter((x) => ciFailure(x, s))) {
              ciFailures.push(`${s.screen}/${s.state} [${p.kind}] ${p.selector} "${p.text.slice(0, 40)}": ${p.detail} (${shot})`);
            }
          }
        }
      }
      for (const message of pageErrors) {
        part.problems.push({
          screen: "app",
          state: "pageerror",
          device: run.device.id,
          platform: run.device.platform,
          locale: run.locale,
          zoom: 100,
          kind: "harnessError",
          severity: "info",
          selector: "window",
          testid: null,
          text: "",
          detail: `page error: ${message.slice(0, 200)}`,
          magnitude: 0,
          screenshot: "",
        });
      }
    } finally {
      part.durationMs = Date.now() - started;
      writePart(part);
      await context.close();
    }

    if (AUDIT) {
      test.info().annotations.push({ type: "layout", description: `${part.captures.length} captures, ${part.problems.length} problems` });
      return;
    }
    expect(part.insets.method, "safe-area insets could not be emulated").toBe("cdp");
    expect(ciFailures, `${ciFailures.length} layout regressions on ${run.device.id} ${run.locale} (see ${out}/summary.md)`).toEqual([]);
  });
}

async function saveShot(page: Page, path: string): Promise<void> {
  mkdirSync(dirname(path), { recursive: true });
  await page.screenshot({ path, type: "jpeg", quality: 70, scale: "css", animations: "disabled", caret: "hide" });
}
