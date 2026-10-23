// DESIGN §11.5: convLink tokens ported (light/dark surfaces, radius, capsules, duration,
// glass without blur on Android, reduced-motion/transparency fallbacks) plus the
// intercom tokens and a brand accent distinct from convLink's green.
import { describe, expect, it } from "vitest";
import { INTERCOM_TOKENS } from "../src/constants.js";
import { declared, readStyle, ruleBodies } from "./helpers.js";

const tokens = readStyle("tokens.css");
const components = readStyle("components.css");
const light = ruleBodies(tokens, ":root").slice(0, 1);
const dark = ruleBodies(tokens, ".dark");

describe("convLink tokens", () => {
  it("light surfaces are #f2f2f7 / #1c1c1e / #ffffff", () => {
    expect(declared(light, "--background")).toBe("#f2f2f7");
    expect(declared(light, "--foreground")).toBe("#1c1c1e");
    expect(declared(light, "--card")).toBe("#ffffff");
  });

  it("dark surfaces are #08080a / #f5f5f7 / #1c1c1e", () => {
    expect(dark.length).toBe(1);
    expect(declared(dark, "--background")).toBe("#08080a");
    expect(declared(dark, "--foreground")).toBe("#f5f5f7");
    expect(declared(dark, "--card")).toBe("#1c1c1e");
  });

  it("radius, capsules and the UI duration", () => {
    expect(declared(light, "--radius")).toBe("1rem");
    expect(declared(light, "--capsule")).toBe("9999px");
    expect(declared(light, "--duration-ui")).toBe("220ms");
  });
});

describe("intercom tokens", () => {
  for (const token of INTERCOM_TOKENS) {
    it(`${token} and its foreground exist in light and dark`, () => {
      for (const theme of [light, dark]) {
        expect(declared(theme, token)).toMatch(/^#[0-9a-f]{6}$/);
        expect(declared(theme, `${token}-foreground`)).toMatch(/^#[0-9a-f]{6}$/);
      }
    });
  }

  it("--status-out is zinc-500", () => {
    expect(declared(light, "--status-out")).toBe("#71717a");
    expect(declared(dark, "--status-out")).toBe("#71717a");
  });

  it("the PTT pulse uses --talk-active", () => {
    expect(tokens).toMatch(/@keyframes mk-talk-pulse[\s\S]*var\(--talk-active\)/);
    expect(declared(ruleBodies(components, '.mk-ptt[data-status="active"]'), "animation")).toContain("mk-talk-pulse");
  });

  it("the brand accent is not green (distinct from convLink)", () => {
    for (const theme of [light, dark]) {
      const hex = declared(theme, "--brand") ?? "";
      const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as [number, number, number];
      expect(g).toBeLessThan(Math.max(r, b));
      expect(hex).not.toBe(declared(theme, "--talk-active"));
    }
  });
});

describe("text contrast of filled intercom colours", () => {
  const luminance = (hex: string) => {
    const channel = (i: number) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
  };
  const ratio = (a: string, b: string) => {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
    return (hi + 0.05) / (lo + 0.05);
  };
  for (const token of [...INTERCOM_TOKENS, "--brand"]) {
    it(`${token} / foreground is at least 4.5:1 (WCAG AA) in both themes`, () => {
      for (const theme of [light, dark]) {
        const bg = declared(theme, token) ?? "";
        const fg = declared(theme, `${token}-foreground`) ?? "";
        expect(ratio(bg, fg), `${token} ${bg} on ${fg}`).toBeGreaterThanOrEqual(4.5);
      }
    });
  }
});

describe("materials and motion fallbacks", () => {
  it("glass blurs only off Android", () => {
    const clean = components.replace(/\/\*[\s\S]*?\*\//g, "");
    const blurRules = [...clean.matchAll(/([^{}]+)\{[^{}]*backdrop-filter:\s*blur/g)].map((m) => (m[1] ?? "").trim());
    expect(blurRules.length).toBeGreaterThan(0);
    for (const selector of blurRules) expect(selector).toContain("html:not(.android)");
  });

  it("reduced transparency and increased contrast drop the blur", () => {
    expect(components).toMatch(/@media \(prefers-reduced-transparency: reduce\), \(prefers-contrast: more\)/);
    expect(declared(ruleBodies(components, ":root.reduce-transparency :is(.glass, .glass-strong)"), "backdrop-filter")).toBe(
      "none",
    );
  });

  it("reduced motion collapses durations and stops the pulse", () => {
    const media = /@media \(prefers-reduced-motion: reduce\)\s*\{\s*:root\s*\{([^}]*)\}/.exec(tokens);
    expect(media?.[1]).toContain("--duration-ui: 0.01ms");
    expect(media?.[1]).toContain("--press-scale: 1");
    expect(declared(ruleBodies(tokens, ":root.reduce-motion"), "--duration-ui")).toBe("0.01ms");
    expect(components).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.mk-ptt\[data-status="active"\][\s\S]*?animation: none/);
  });
});
