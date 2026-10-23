// Run configuration rules (WP-21 acceptance): a run cannot start without an audio path
// and an anchor, T1/T3 default to the `audio` anchor, variants map onto LabParams
// (role swap, headset → full audio, mixable session, traffic pattern), and everything
// else goes into the bounded variant label.
import { isUiCommand } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import type { BenchSpec } from "./catalog";
import { BENCH_CATALOG, benchSpec } from "./catalog";
import { defaultDraft, needsLocation, toLabParams, variantLabel, withAnchor, withAxis } from "./config";

function spec(bench: Parameters<typeof benchSpec>[0]): BenchSpec {
  const s = benchSpec(bench);
  if (s === undefined) throw new Error(`no ${bench}`);
  return s;
}

describe("defaultDraft", () => {
  it("pre-selects the catalog audio path and anchor, GPS off", () => {
    for (const s of BENCH_CATALOG) {
      const d = defaultDraft(s);
      expect(d.audio, s.bench).toBe(s.audio);
      expect(d.anchor, s.bench).toBe(s.anchor);
      expect(d.gps).toBe(false);
      expect(toLabParams(s, d).ok, s.bench).toBe(true);
    }
  });

  it("T1 and T3 default to the audio anchor", () => {
    expect(defaultDraft(spec("T1")).anchor).toBe("audio");
    expect(defaultDraft(spec("T3")).anchor).toBe("audio");
    expect(defaultDraft(spec("T1")).audio).toBe("renderOnly");
    expect(defaultDraft(spec("T3")).audio).toBe("renderOnly");
  });

  it("uses the duration of the selected traffic pattern", () => {
    const t0 = spec("T0");
    const d = defaultDraft(t0);
    expect(d.axes["pattern"]).toBe("saturate");
    expect(d.durationS).toBe(20);
    expect(withAxis(t0, d, "pattern", "voice2").durationS).toBe(60);
    expect(defaultDraft(spec("T2")).durationS).toBe(14_400);
  });
});

describe("toLabParams", () => {
  it("refuses a draft without an audio path or an anchor", () => {
    const t1 = spec("T1");
    const d = defaultDraft(t1);
    expect(toLabParams(t1, { ...d, audio: null })).toEqual({ ok: false, problem: "audioMissing" });
    expect(toLabParams(t1, { ...d, anchor: null })).toEqual({ ok: false, problem: "anchorMissing" });
    expect(toLabParams(t1, { ...d, anchor: "none" })).toEqual({ ok: false, problem: "anchorNotAllowed" });
  });

  it("always carries audio, anchor and gps, and is a valid labStart", () => {
    for (const s of BENCH_CATALOG) {
      const check = toLabParams(s, defaultDraft(s));
      if (!check.ok) throw new Error(`${s.bench}: ${check.problem}`);
      expect(check.params.audio).toBe(s.audio);
      expect(check.params.anchor).toBe(s.anchor);
      expect(check.params.gps).toBe(false);
      expect(isUiCommand({ type: "labStart", bench: s.bench, params: check.params }), s.bench).toBe(true);
    }
  });

  it("maps T0 variants: transport, role swap, headset (full audio) and traffic", () => {
    const t0 = spec("T0");
    let d = defaultDraft(t0);
    d = withAxis(t0, d, "transport", "gattWrite");
    d = withAxis(t0, d, "role", "peripheral");
    d = withAxis(t0, d, "headset", "hfp");
    d = withAxis(t0, d, "pattern", "voice4");
    d = withAxis(t0, d, "distance", "far");
    expect(d.audio).toBe("full");
    const check = toLabParams(t0, d);
    expect(check).toEqual({
      ok: true,
      params: {
        audio: "full",
        anchor: "audio",
        gps: false,
        durationS: 60,
        pattern: "voiceLike",
        streams: 4,
        transport: "gattWrite",
        role: "peripheral",
        headset: true,
        variant: "distance:far",
      },
    });
    expect(withAxis(t0, d, "headset", "off").audio).toBe("off");
  });

  it("refuses a headset variant without the full audio path", () => {
    const t0 = spec("T0");
    const d = withAxis(t0, defaultDraft(t0), "headset", "hfp");
    expect(toLabParams(t0, { ...d, audio: "renderOnly" })).toEqual({ ok: false, problem: "headsetNeedsFull" });
  });

  it("omits stream counts for patterns without voice streams", () => {
    const t0 = spec("T0");
    const check = toLabParams(t0, withAxis(t0, defaultDraft(t0), "pattern", "ping"));
    expect(check.ok && check.params.streams).toBeUndefined();
    expect(check.ok && check.params.pattern).toBe("ping");
  });

  it("maps the T2 session variant to mixable and the anchor axis to the anchor", () => {
    const t2 = spec("T2");
    let d = withAxis(t2, defaultDraft(t2), "session", "mixable");
    d = withAxis(t2, d, "anchor", "none");
    const check = toLabParams(t2, d);
    expect(check.ok && check.params.mixable).toBe(true);
    expect(check.ok && check.params.anchor).toBe("none");
    expect(withAnchor(t2, d, "audioLocation").axes["anchor"]).toBe("audioLocation");
  });

  it("ignores unknown axes and values", () => {
    const t0 = spec("T0");
    const d = defaultDraft(t0);
    expect(withAxis(t0, d, "transport", "carrierPigeon")).toBe(d);
    expect(withAxis(t0, d, "mount", "pocket")).toBe(d);
  });

  it("checks the duration range", () => {
    const t0 = spec("T0");
    const d = defaultDraft(t0);
    for (const bad of [0, -1, 1.5, Number.NaN, 86_401]) {
      expect(toLabParams(t0, { ...d, durationS: bad }), String(bad)).toEqual({ ok: false, problem: "durationInvalid" });
    }
    expect(toLabParams(t0, { ...d, durationS: 5 }).ok).toBe(true);
  });

  it("records non-parameter axes and the note in a ≤ 64 byte variant label", () => {
    const t3 = spec("T3");
    const d = { ...defaultDraft(t3), note: "  three  phones " };
    expect(variantLabel(t3, d)).toBe("state:bothFg;break:blacklist30s;three phones");
    expect(variantLabel(spec("T8"), defaultDraft(spec("T8")))).toBeNull();
    const long = { ...d, note: "ñ".repeat(20) };
    expect(toLabParams(t3, long)).toEqual({ ok: false, problem: "variantTooLong" });
  });
});

describe("needsLocation", () => {
  it("is true for GPS consent or the location anchor", () => {
    expect(needsLocation({ gps: false, anchor: "audio" })).toBe(false);
    expect(needsLocation({ gps: true, anchor: "audio" })).toBe(true);
    expect(needsLocation({ gps: false, anchor: "audioLocation" })).toBe(true);
  });
});
