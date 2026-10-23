// Drift check of the Field test bench catalog against the WP-05 catalog in
// crates/mesh-lab/src/catalog.rs (phase, phones, default audio path and anchor, allowed
// anchors, default traffic, variant axes, repetitions, T0 pattern axis), plus the rules
// the UI relies on (no `none` default; T1/T3 default to the `audio` anchor).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { BENCH_CATALOG, FIELD_TEST_BENCHES, PARAM_AXES, audioFor, benchSpec, patternAxis } from "./catalog";
import { repoFile } from "./testing";

const RUST = readFileSync(repoFile("crates/mesh-lab/src/catalog.rs"), "utf8");

const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);
const num = (s: string) => Number(s.replace(/_/g, ""));

/** `const NAME: &[&str] = &["a", "b"];` and `const NAME: &[Anchor] = &[Anchor::A, …];` */
function constArrays(): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const m of RUST.matchAll(/const (\w+): &\[[^\]]+\] = &\[([^\]]*)\];/g)) {
    const items = [...(m[2] ?? "").matchAll(/"([^"]*)"|Anchor::(\w+)/g)].map((x) => x[1] ?? lowerFirst(x[2] ?? ""));
    map.set(m[1] ?? "", items);
  }
  return map;
}

/** The `Defaults { … }` constants by name. */
function constDefaults(): Map<string, string> {
  const map = new Map<string, string>();
  for (const m of RUST.matchAll(/const (\w+): Defaults = (Defaults \{[^}]*\});/g)) map.set(m[1] ?? "", m[2] ?? "");
  return map;
}

interface RustBench {
  bench: string;
  phase: number;
  devices: [number, number];
  audio: string;
  anchor: string;
  anchors: string[];
  defaults: { durationS: number; pattern: string | null; streams: number; payloadB: number; intervalMs: number; pingHz: number };
  axes: { key: string; values: string[] }[];
  repetitions: number;
}

function parseRustCatalog(): Map<string, RustBench> {
  const arrays = constArrays();
  const defaultsByName = constDefaults();
  const body = RUST.slice(RUST.indexOf("pub const CATALOG"));
  const blocks = body.split(/\n {4}BenchSpec \{/).slice(1);
  const out = new Map<string, RustBench>();
  for (const block of blocks) {
    const pick = (re: RegExp) => {
      const m = re.exec(block);
      if (m === null) throw new Error(`catalog.rs: ${re} not found`);
      return m;
    };
    const bench = lowerFirst(pick(/bench: Bench::(\w+)/)[1] ?? "");
    const benchId = bench.startsWith("t") ? bench.toUpperCase().replace(/([0-9])([A-Z])$/, (_m, d: string, l: string) => d + l.toLowerCase()) : bench;
    const anchorsRef = pick(/anchors: (\w+),/)[1] ?? "";
    const defaultsText = /defaults: (\w+),/.exec(block)?.[1];
    const d = defaultsText !== undefined ? (defaultsByName.get(defaultsText) ?? "") : pick(/defaults: (Defaults \{[^}]*\})/)[1] ?? "";
    const field = (name: string) => num(new RegExp(`${name}: ([\\d_]+)`).exec(d)?.[1] ?? "NaN");
    const pattern = lowerFirst(/pattern: Pattern::(\w+)/.exec(d)?.[1] ?? "");
    const axesText = pick(/axes: &\[([\s\S]*?)\n {8}\}?\],|axes: &\[\],/)[1] ?? "";
    const axes = [...axesText.matchAll(/key: "(\w+)",\s*values: (?:(\w+)|&\[([^\]]*)\])/g)].map((m) => ({
      key: m[1] ?? "",
      values: m[2] !== undefined ? (arrays.get(m[2]) ?? []) : [...(m[3] ?? "").matchAll(/"([^"]*)"/g)].map((x) => x[1] ?? ""),
    }));
    const devices = pick(/devices: \((\d+), (\d+)\)/);
    out.set(benchId, {
      bench: benchId,
      phase: num(pick(/phase: (\d+),\n {8}title/)[1] ?? ""),
      devices: [num(devices[1] ?? ""), num(devices[2] ?? "")],
      audio: lowerFirst(pick(/\n {8}audio: Audio::(\w+)/)[1] ?? ""),
      anchor: lowerFirst(pick(/\n {8}anchor: Anchor::(\w+)/)[1] ?? ""),
      anchors: arrays.get(anchorsRef) ?? [],
      defaults: {
        durationS: field("duration_s"),
        pattern: pattern === "idle" ? null : pattern,
        streams: field("streams"),
        payloadB: field("payload_b"),
        intervalMs: field("interval_ms"),
        pingHz: field("ping_hz"),
      },
      axes,
      repetitions: num(pick(/repetitions: ([\d_]+)/)[1] ?? ""),
    });
  }
  return out;
}

describe("bench catalog mirrors crates/mesh-lab/src/catalog.rs", () => {
  const rust = parseRustCatalog();

  it("parses the Rust catalog", () => {
    expect([...rust.keys()]).toEqual(expect.arrayContaining(["T0", "T0b", "T0c", "T1", "T2", "T3", "T5a", "T5b", "T6", "T8", "voxCal"]));
  });

  for (const spec of BENCH_CATALOG) {
    it(`${spec.bench} matches`, () => {
      const r = rust.get(spec.bench);
      expect(r, spec.bench).toBeDefined();
      expect({
        bench: spec.bench,
        phase: spec.phase,
        devices: [...spec.devices],
        audio: spec.audio,
        anchor: spec.anchor,
        anchors: [...spec.anchors],
        defaults: { ...spec.defaults },
        axes: spec.axes.map((a) => ({ key: a.key, values: [...a.values] })),
        repetitions: spec.repetitions,
      }).toEqual(r);
    });
  }

  it("T0 pattern axis matches pattern_axis()", () => {
    for (const m of RUST.matchAll(/"(\w+)" => \(Pattern::(\w+), (\d+), (\d+)\)/g)) {
      expect(patternAxis(m[1] ?? ""), m[1]).toEqual({
        pattern: lowerFirst(m[2] ?? ""),
        streams: Number(m[3]),
        durationS: Number(m[4]),
      });
    }
    expect(patternAxis("bogus")).toBeUndefined();
  });
});

describe("Field test bench list", () => {
  it("lists the Phase 0 benches first, then the Phase 1 additions", () => {
    expect(FIELD_TEST_BENCHES).toEqual(["T0", "T0b", "T0c", "T1", "T2", "T3", "T5a", "T6", "T5b", "T8", "voxCal"]);
    expect(BENCH_CATALOG.filter((b) => b.phase === 1).map((b) => b.bench)).toEqual(["T5b", "T8", "voxCal"]);
  });

  it("never defaults to the `none` anchor and offers the default first", () => {
    for (const spec of BENCH_CATALOG) {
      expect(spec.anchor).not.toBe("none");
      expect(spec.anchors[0]).toBe(spec.anchor);
    }
  });

  it("T1 and T3 default to the `audio` anchor; T2 offers `none` as a control", () => {
    expect(benchSpec("T1")?.anchor).toBe("audio");
    expect(benchSpec("T3")?.anchor).toBe("audio");
    expect(benchSpec("T2")?.anchors).toContain("none");
    expect(benchSpec("T4")).toBeUndefined();
  });

  it("switches to the full audio path for headset variants", () => {
    const t0 = benchSpec("T0");
    if (t0 === undefined) throw new Error("T0 missing");
    expect(audioFor(t0, false)).toBe("off");
    expect(audioFor(t0, true)).toBe("full");
  });

  it("maps only known axes to typed parameters", () => {
    expect([...PARAM_AXES].sort()).toEqual(["anchor", "headset", "pattern", "role", "session", "transport"]);
  });
});
