// QualityBars, Sparkline and the runtime theme helpers.
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { QualityBars, clampBars, qualityTone } from "../src/components/QualityBars.js";
import { Sparkline, sparkDomain, sparklinePath } from "../src/components/Sparkline.js";
import { applyTheme, resolveTheme, setAccessibilityClasses, setPlatformClass } from "../src/theme.js";

describe("QualityBars", () => {
  it("clamps and rounds bar counts", () => {
    expect(clampBars(2.6, 4)).toBe(3);
    expect(clampBars(9, 4)).toBe(4);
    expect(clampBars(-1, 4)).toBe(0);
    expect(clampBars(Number.NaN, 4)).toBe(0);
  });

  it("maps bars to tones", () => {
    expect([0, 1, 2, 3, 4].map((n) => qualityTone(n))).toEqual(["none", "poor", "fair", "good", "good"]);
    expect(qualityTone(1, 3)).toBe("poor");
    expect(qualityTone(2, 3)).toBe("fair");
  });

  it("renders max bars, fills the first n and exposes the label", () => {
    render(<QualityBars bars={2} label="Link quality: fair" />);
    const meter = screen.getByRole("img", { name: "Link quality: fair" });
    expect(meter.dataset["tone"]).toBe("fair");
    const bars = [...meter.children] as HTMLElement[];
    expect(bars.map((b) => b.dataset["filled"])).toEqual(["true", "true", "false", "false"]);
    expect(bars.map((b) => b.style.height)).toEqual(["25%", "50%", "75%", "100%"]);
  });
});

describe("Sparkline", () => {
  it("domain ignores gaps, honours overrides and widens flat data", () => {
    expect(sparkDomain([3, null, 1, Number.NaN, 5])).toEqual({ min: 1, max: 5 });
    expect(sparkDomain([3, 4], 0, 10)).toEqual({ min: 0, max: 10 });
    expect(sparkDomain([2, 2])).toEqual({ min: 1, max: 3 });
    expect(sparkDomain([])).toEqual({ min: 0, max: 1 });
  });

  it("lays samples out evenly with an inverted value axis and breaks on gaps", () => {
    expect(sparklinePath([0, 10, 5], 100, 20, { min: 0, max: 10 })).toBe("M0 20 L50 0 L100 10");
    expect(sparklinePath([0, null, 10, 10], 30, 10, { min: 0, max: 10 })).toBe("M0 10 M20 0 L30 0");
    expect(sparklinePath([20, -5], 10, 10, { min: 0, max: 10 })).toBe("M0 0 L10 10");
    expect(sparklinePath([4], 10, 10, { min: 0, max: 8 })).toBe("M5 5");
    expect(sparklinePath([], 10, 10)).toBe("");
  });

  it("renders the line, the threshold and the accessible summary", () => {
    const { container } = render(<Sparkline values={[0, 4]} label="Loss, last 60 s" width={40} height={10} min={0} max={8} threshold={2} />);
    expect(screen.getByRole("img", { name: "Loss, last 60 s" })).toBeTruthy();
    expect(container.querySelector(".mk-sparkline-line")?.getAttribute("d")).toBe("M0 10 L40 5");
    expect(container.querySelector(".mk-sparkline-threshold")?.getAttribute("y1")).toBe("7.5");
  });

  it("draws no path for empty data", () => {
    const { container } = render(<Sparkline values={[]} label="No data" />);
    expect(container.querySelector("path")).toBeNull();
  });
});

describe("theme helpers", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("resolveTheme keeps explicit themes and defaults system to dark", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("system")).toBe("dark");
  });

  it("applyTheme toggles .dark, data-theme and color-scheme", () => {
    const root = document.createElement("html");
    applyTheme("dark", root);
    expect(root.classList.contains("dark")).toBe(true);
    expect(root.dataset["theme"]).toBe("dark");
    expect(root.style.colorScheme).toBe("dark");
    applyTheme("light", root);
    expect(root.classList.contains("dark")).toBe(false);
    expect(root.dataset["theme"]).toBe("light");
  });

  it("system follows the media query until stopped", () => {
    const listeners = new Set<(e: { matches: boolean }) => void>();
    const query = {
      matches: false,
      addEventListener: (_: string, fn: (e: { matches: boolean }) => void) => listeners.add(fn),
      removeEventListener: (_: string, fn: (e: { matches: boolean }) => void) => listeners.delete(fn),
    };
    vi.stubGlobal("matchMedia", vi.fn(() => query));
    const root = document.createElement("html");
    const stop = applyTheme("system", root);
    expect(root.dataset["theme"]).toBe("light");
    for (const fn of listeners) fn({ matches: true });
    expect(root.dataset["theme"]).toBe("dark");
    stop();
    expect(listeners.size).toBe(0);
  });

  it("platform and accessibility classes", () => {
    const root = document.createElement("html");
    setPlatformClass("android", root);
    expect(root.className).toBe("android");
    setPlatformClass("ios", root);
    expect(root.className).toBe("ios");
    setAccessibilityClasses({ reduceMotion: true, reduceTransparency: true }, root);
    expect(root.classList.contains("reduce-motion")).toBe(true);
    expect(root.classList.contains("reduce-transparency")).toBe(true);
    setAccessibilityClasses({ reduceMotion: false }, root);
    expect(root.classList.contains("reduce-motion")).toBe(false);
    expect(root.classList.contains("reduce-transparency")).toBe(true);
  });
});
