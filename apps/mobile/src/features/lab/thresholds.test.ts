// The thresholds the Field test shows are exactly tools/lab-analyze/thresholds.json
// (WP-05), and the criterion copy of every listed bench gets the params it interpolates.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { catalogTexts } from "../../i18n/index";
import { FIELD_TEST_BENCHES } from "./catalog";
import { LAB_THRESHOLDS, criterionParams } from "./thresholds";
import { repoFile } from "./testing";

const JSON_PATH = repoFile("tools/lab-analyze/thresholds.json");

describe("LAB_THRESHOLDS", () => {
  it("equals tools/lab-analyze/thresholds.json", () => {
    const { $comment: _comment, ...file } = JSON.parse(readFileSync(JSON_PATH, "utf8")) as Record<string, unknown>;
    expect(LAB_THRESHOLDS).toEqual(file);
  });

  it("formats ratios as percentages for the copy", () => {
    expect(criterionParams("T0")).toEqual({ streams: 2, lossPct: 5, owdMs: 60, fastKbps: 150 });
    expect(criterionParams("T1")).toEqual({ sharePct: 95, lossPct: 5, gapM: 50 });
    expect(criterionParams("T6")).toEqual({ pctPerH: 5, minMin: 30 });
    expect(criterionParams("T5b")).toEqual({});
  });

  for (const language of ["en", "es"] as const) {
    it(`every listed bench's criterion (${language}) has its params and no missing placeholder`, () => {
      const texts = catalogTexts("lab", language);
      for (const bench of FIELD_TEST_BENCHES) {
        const text = texts[`bench.${bench}.criterion`];
        expect(text, bench).toBeTruthy();
        const wanted = [...(text ?? "").matchAll(/\{\{\s*(\w+)\s*\}\}/g)].map((m) => m[1]).sort();
        expect(wanted, bench).toEqual(Object.keys(criterionParams(bench)).sort());
      }
    });
  }
});
