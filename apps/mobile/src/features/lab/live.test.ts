// Live-view helpers: 60 s windows over the 2 Hz labProgress ring, metric series with
// gaps, anchor-drop detection and the elapsed clock.
import type { LabLiveSample } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import type { LabSamplePoint } from "../../store/useLabStore";
import { anchorDropped, clock, lastWindow, metricValue, series } from "./live";

const sample = (over: Partial<LabLiveSample> = {}): LabLiveSample => ({
  links: 1,
  peers: 1,
  txKbps: 50,
  rxKbps: 49,
  loss: 0.01,
  rttP50Ms: 30,
  rttP95Ms: 45,
  rssi: -60,
  distM: null,
  stalls: 0,
  anchorOk: true,
  ...over,
});

const ring = (n: number, every = 0.5): LabSamplePoint[] =>
  Array.from({ length: n }, (_, i) => ({ elapsedS: (i + 1) * every, sample: sample({ txKbps: i }) }));

describe("live helpers", () => {
  it("keeps the last 60 s of samples", () => {
    expect(lastWindow([])).toEqual([]);
    const points = ring(300); // 150 s at 2 Hz
    const w = lastWindow(points);
    expect(w[w.length - 1]?.elapsedS).toBe(150);
    expect(w[0]?.elapsedS).toBe(90.5);
    expect(w).toHaveLength(120);
    expect(lastWindow(ring(10))).toHaveLength(10);
  });

  it("returns gaps for missing values", () => {
    const points: LabSamplePoint[] = [
      { elapsedS: 0.5, sample: sample({ rttP95Ms: null }) },
      { elapsedS: 1, sample: sample({ rttP95Ms: 40 }) },
    ];
    expect(series(points, "rttP95Ms")).toEqual([null, 40]);
    expect(series(points, "distM")).toEqual([null, null]);
    expect(metricValue(sample({ loss: Number.NaN }), "loss")).toBeNull();
  });

  it("detects a dropped anchor in the window", () => {
    const points = ring(4);
    expect(anchorDropped(points)).toBe(false);
    points[1] = { elapsedS: 1, sample: sample({ anchorOk: false }) };
    expect(anchorDropped(points)).toBe(true);
  });

  it("formats the elapsed clock", () => {
    expect(clock(0)).toBe("00:00");
    expect(clock(65.9)).toBe("01:05");
    expect(clock(3725)).toBe("1:02:05");
    expect(clock(-3)).toBe("00:00");
  });
});
