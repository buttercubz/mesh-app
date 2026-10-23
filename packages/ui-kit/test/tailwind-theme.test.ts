// @vitest-environment node
// The Tailwind v4 theme export (theme.css) compiles with the real Tailwind compiler and
// maps the tokens to utilities that reference the runtime variables (DESIGN §11.5).
import { compile } from "@tailwindcss/node";
import { beforeAll, describe, expect, it } from "vitest";
import { INTERCOM_TOKENS } from "../src/constants.js";
import { stylesDir } from "./helpers.js";

let build: (candidates: string[]) => string;

beforeAll(async () => {
  const compiler = await compile('@import "tailwindcss";\n@import "./theme.css";\n', {
    base: stylesDir,
    onDependency: () => {},
  });
  build = (candidates) => compiler.build(candidates);
});

/** Body of the rule generated for `.className` (escaped as Tailwind prints it). */
function utility(css: string, className: string): string {
  const escaped = className.replace(/[:]/g, "\\\\:");
  const re = new RegExp(`\\.${escaped}\\s*\\{([^}]*)\\}`);
  return re.exec(css)?.[1] ?? "";
}

describe("Tailwind v4 theme export", () => {
  it("maps every intercom colour and its foreground to a colour utility", () => {
    const names = INTERCOM_TOKENS.flatMap((t) => [t.slice(2), `${t.slice(2)}-foreground`]);
    const css = build(names.map((n) => `bg-${n}`));
    for (const name of names) {
      expect(utility(css, `bg-${name}`), name).toContain(`background-color: var(--${name})`);
    }
  });

  it("maps the brand accent and the convLink surfaces", () => {
    const css = build(["bg-brand", "text-brand-foreground", "bg-card", "text-muted-foreground", "border-separator"]);
    expect(utility(css, "bg-brand")).toContain("var(--brand)");
    expect(utility(css, "text-brand-foreground")).toContain("var(--brand-foreground)");
    expect(utility(css, "bg-card")).toContain("var(--card)");
    expect(utility(css, "text-muted-foreground")).toContain("var(--muted-foreground)");
    expect(utility(css, "border-separator")).toContain("var(--separator)");
  });

  it("exposes the driving targets, capsules, radius, shadows and motion", () => {
    const css = build([
      "min-h-target",
      "size-ptt",
      "rounded-capsule",
      "rounded-lg",
      "shadow-chrome",
      "animate-talk-pulse",
      "duration-ui",
      "ease-ui",
      "font-sans",
    ]);
    expect(utility(css, "min-h-target")).toContain("var(--target-min)");
    expect(utility(css, "size-ptt")).toContain("var(--ptt-size)");
    expect(utility(css, "rounded-capsule")).toContain("var(--capsule)");
    expect(utility(css, "rounded-lg")).toContain("var(--radius)");
    expect(utility(css, "shadow-chrome")).toContain("var(--elevation-chrome)");
    expect(utility(css, "animate-talk-pulse")).toContain("mk-talk-pulse");
    expect(utility(css, "duration-ui")).toContain("transition-duration: var(--duration-ui)");
    expect(utility(css, "ease-ui")).toContain("var(--ease-ui)");
    expect(utility(css, "font-sans")).toContain("var(--font-family-ui)");
  });

  it("dark: and android: are class-based variants", () => {
    const css = build(["dark:bg-brand", "android:hidden"]);
    expect(css).toContain('.dark\\:bg-brand:where(.dark, .dark *, [data-theme="dark"], [data-theme="dark"] *) {');
    expect(css).toContain(".android\\:hidden:where(.android, .android *) {");
  });

  it("includes the tokens and the component layer", () => {
    const css = build([]);
    expect(css).toContain("--talk-active: #208136");
    expect(css).toContain("@layer components");
    expect(css).toContain(".mk-ptt");
  });
});
