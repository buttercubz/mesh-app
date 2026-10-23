import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { fireEvent } from "@testing-library/react";

// Plain paths, not URL objects: under the jsdom environment the global URL is jsdom's,
// which node:fs does not accept as a file URL.
export const stylesDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../src/styles");

export function readStyle(name: string): string {
  return readFileSync(path.join(stylesDir, name), "utf8");
}

/** Dispatches a PointerEvent with a pointerId (jsdom drops unknown init fields otherwise). */
export function pointer(
  el: Element,
  type: "pointerDown" | "pointerUp" | "pointerCancel" | "lostPointerCapture",
  init: { pointerId?: number; button?: number; pointerType?: string } = {},
): void {
  const { pointerId = 1, button = 0, pointerType = "touch" } = init;
  fireEvent[type](el, { pointerId, button, pointerType, isPrimary: pointerId === 1 });
}

/** Sets document.visibilityState and fires visibilitychange. */
export function setVisibility(state: "hidden" | "visible"): void {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  document.dispatchEvent(new Event("visibilitychange"));
}

/**
 * Rule bodies of `selector` in a stylesheet (exact selector text match, any layer).
 * Comments are stripped first.
 */
export function ruleBodies(css: string, selector: string): string[] {
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const out: string[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  for (const m of clean.matchAll(re)) {
    const selectors = splitSelectorList(m[1] ?? "");
    if (selectors.includes(selector)) out.push(m[2] ?? "");
  }
  return out;
}

/** Splits a selector list on top-level commas (not the ones inside `:is(...)` etc.). */
export function splitSelectorList(list: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let current = "";
  for (const ch of list) {
    if (ch === "(") depth += 1;
    else if (ch === ")") depth -= 1;
    if (ch === "," && depth === 0) {
      out.push(current.trim());
      current = "";
    } else {
      current += ch;
    }
  }
  out.push(current.trim());
  return out;
}

/** Value of `prop` declared in the first rule body that has it. */
export function declared(bodies: string[], prop: string): string | undefined {
  for (const body of bodies) {
    for (const decl of body.split(";")) {
      const idx = decl.indexOf(":");
      if (idx < 0) continue;
      if (decl.slice(0, idx).trim() === prop) return decl.slice(idx + 1).trim();
    }
  }
  return undefined;
}
