// `lab` catalog (WP-21 acceptance: ES/EN parity): both languages have exactly the same
// keys and placeholders, and every key the Field test builds at runtime exists (bench
// copy, variant axes and values, audio paths, anchors, config problems, verdicts, mark
// presets, procedure notes, live metrics).
import { LAB_ANCHORS, LAB_AUDIOS, LAB_VERDICTS } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import { APP_CATALOGS } from "../../app/i18n";
import { catalogParityErrors, translate } from "../../i18n/index";
import { labCatalog } from "../../i18n/catalogs/lab";
import { BENCH_CATALOG, FIELD_TEST_BENCHES } from "./catalog";
import type { DraftProblem } from "./config";
import { SPARK_METRICS } from "./live";
import { MARK_PRESETS } from "./MarkSheet";

const PROBLEMS: readonly DraftProblem[] = [
  "audioMissing",
  "anchorMissing",
  "anchorNotAllowed",
  "headsetNeedsFull",
  "durationInvalid",
  "variantTooLong",
];

const placeholders = (text: string) => [...text.matchAll(/\{\{\s*([\w.]+)\s*\}\}/g)].map((m) => m[1]).sort();

function runtimeKeys(): string[] {
  const keys: string[] = [];
  for (const bench of [...FIELD_TEST_BENCHES, "T4", "ride"]) {
    for (const part of ["title", "setup", "captured", "criterion"]) keys.push(`bench.${bench}.${part}`);
  }
  for (const bench of FIELD_TEST_BENCHES) keys.push(`procedure.note.${bench}`);
  for (const spec of BENCH_CATALOG) {
    for (const axis of spec.axes) {
      keys.push(`axis.${axis.key}`);
      for (const v of axis.values) keys.push(`axis.${axis.key}.${v}`);
    }
  }
  for (const a of LAB_AUDIOS) keys.push(`audio.${a}`, `audio.${a}.hint`);
  for (const a of LAB_ANCHORS) keys.push(`anchor.${a}`, `anchor.${a}.hint`);
  for (const p of PROBLEMS) keys.push(`config.problem.${p}`);
  for (const v of [...LAB_VERDICTS, "interrupted"]) keys.push(`verdict.${v}`);
  for (const m of MARK_PRESETS) keys.push(`mark.preset.${m}`);
  for (const m of SPARK_METRICS) keys.push(`live.${m}`);
  return keys;
}

describe("lab catalog", () => {
  it("is registered as the `lab` namespace", () => {
    expect(APP_CATALOGS["lab"]).toBe(labCatalog);
  });

  it("has ES/EN parity with the same placeholders", () => {
    expect(catalogParityErrors(labCatalog)).toEqual([]);
    expect(Object.keys(labCatalog.es).sort()).toEqual(Object.keys(labCatalog.en).sort());
    for (const [key, en] of Object.entries(labCatalog.en)) {
      expect(placeholders(labCatalog.es[key as keyof typeof labCatalog.es]), key).toEqual(placeholders(en));
    }
  });

  it("holds every key the Field test builds at runtime", () => {
    const en = labCatalog.en as Record<string, string>;
    const missing = runtimeKeys().filter((k) => !Object.hasOwn(en, k));
    expect(missing).toEqual([]);
  });

  it("labels the `none` anchor as a suspension control in both languages", () => {
    expect(translate("lab", "anchor.none.hint", {}, "en")).toMatch(/^Control: the phone may suspend/);
    expect(translate("lab", "anchor.none.hint", {}, "es")).toMatch(/^Control: el teléfono puede suspenderse/);
  });

  it("carries the passenger/stop safety note in both languages", () => {
    expect(translate("lab", "mark.safety", {}, "en")).toContain("passengers or at stops");
    expect(translate("lab", "mark.safety", {}, "es")).toContain("acompañantes o en paradas");
  });
});
