// Acceptance "44 px minimum targets" (DESIGN §11.4): every interactive primitive's
// class declares a minimum size of var(--target-min) (= 44 px) or the PTT size, and the
// rendered primitives actually carry those classes.
import { render, screen } from "@testing-library/react";
import { createElement } from "react";
import { describe, expect, it } from "vitest";
import { Banner, BigPttButton, Button, ListRow, SegmentedControl, Sheet, Toast } from "../src/index.js";
import { PTT_SIZE_PX, TARGET_MIN_PX } from "../src/constants.js";
import { declared, readStyle, ruleBodies } from "./helpers.js";

const tokens = readStyle("tokens.css");
const components = readStyle("components.css");

function px(value: string | undefined): number {
  if (!value) return Number.NaN;
  const resolved = value.replace(/var\((--[\w-]+)\)/g, (_, name: string) => declared(ruleBodies(tokens, ":root"), name) ?? "");
  const m = /^(\d+(?:\.\d+)?)px$/.exec(resolved.trim());
  return m ? Number(m[1]) : Number.NaN;
}

describe("touch target tokens", () => {
  it("define 44 px and 160 px", () => {
    const root = ruleBodies(tokens, ":root");
    expect(declared(root, "--target-min")).toBe(`${TARGET_MIN_PX}px`);
    expect(declared(root, "--ptt-size")).toBe(`${PTT_SIZE_PX}px`);
  });

  const interactive = [".mk-button", ".mk-segment", ".mk-list-row--interactive", ".mk-toast-close", ".mk-ptt"];
  for (const selector of interactive) {
    it(`${selector} has a min-height and min-width of at least 44 px`, () => {
      const bodies = ruleBodies(components, selector);
      expect(bodies.length).toBeGreaterThan(0);
      expect(px(declared(bodies, "min-height"))).toBeGreaterThanOrEqual(TARGET_MIN_PX);
      // List rows span the full width; every other target is at least 44 wide too.
      if (selector !== ".mk-list-row--interactive") {
        expect(px(declared(bodies, "min-width"))).toBeGreaterThanOrEqual(TARGET_MIN_PX);
      }
    });
  }

  it("the PTT button is 160 px square", () => {
    const bodies = ruleBodies(components, ".mk-ptt");
    for (const prop of ["width", "height", "min-width", "min-height"]) {
      expect(px(declared(bodies, prop))).toBe(PTT_SIZE_PX);
    }
  });

  it("the icon button is exactly 44 px square", () => {
    const bodies = ruleBodies(components, ".mk-button--icon");
    expect(px(declared(bodies, "width"))).toBe(TARGET_MIN_PX);
    expect(px(declared(bodies, "height"))).toBe(TARGET_MIN_PX);
  });

  it("no component rule lowers a min-height below 44 px", () => {
    const clean = components.replace(/\/\*[\s\S]*?\*\//g, "");
    for (const m of clean.matchAll(/min-height:\s*([^;]+);/g)) {
      const value = px(m[1]);
      if (Number.isFinite(value)) expect(value).toBeGreaterThanOrEqual(TARGET_MIN_PX);
    }
  });
});

describe("rendered interactive primitives carry a 44 px target class", () => {
  it("every button rendered by the kit uses a sized class", () => {
    render(
      createElement(
        "div",
        null,
        createElement(Button, null, "Go live"),
        createElement(BigPttButton, { mode: "hold", onDown: () => {}, onUp: () => {}, level: 0, label: "Talk" }),
        createElement(SegmentedControl<"vox" | "ptt">, {
          label: "Mode",
          value: "vox",
          onChange: () => {},
          options: [
            { value: "vox", label: "VOX" },
            { value: "ptt", label: "PTT" },
          ],
        }),
        createElement(ListRow, { title: "Members", onClick: () => {} }),
        createElement(Banner, { title: "Ladder", action: { label: "Resume", onClick: () => {} } }),
        createElement(Toast, { title: "Hi", dismissLabel: "Dismiss", onDismiss: () => {} }),
        createElement(Sheet, { open: true, onClose: () => {}, title: "Sheet", closeLabel: "Close", children: "x" }),
      ),
    );
    const sized = ["mk-button", "mk-ptt", "mk-segment", "mk-list-row--interactive", "mk-toast-close"];
    const buttons = screen.getAllByRole("button", { hidden: true }).concat(screen.getAllByRole("radio"));
    expect(buttons.length).toBeGreaterThanOrEqual(8);
    for (const b of buttons) {
      expect(sized.some((c) => b.classList.contains(c)), b.outerHTML).toBe(true);
    }
  });
});
