// iOS Dynamic Type mapping: same 100-130 % clamp as the Android WebView text zoom.
import { describe, expect, it } from "vitest";

import { textScaleFor } from "./textSize";

describe("textScaleFor", () => {
  it("maps the iOS body size to a root scale clamped to 100-130 %", () => {
    expect(textScaleFor(17)).toBe(1);
    expect(textScaleFor(14)).toBe(1);
    expect(textScaleFor(19)).toBeCloseTo(19 / 17);
    expect(textScaleFor(23)).toBe(1.3);
    expect(textScaleFor(53)).toBe(1.3);
    expect(textScaleFor(Number.NaN)).toBe(1);
  });
});
