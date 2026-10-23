// Go / no-go thresholds shown by the Field test (WP-21, DESIGN §12.2, §12.5). Mirror of
// tools/lab-analyze/thresholds.json (owned by WP-05; crates/mesh-lab/src/verdict.rs
// mirrors the same numbers for the on-device verdict). thresholds.test.ts compares this
// object with the JSON file, so a changed threshold fails the tests instead of drifting.
import type { LabBench } from "@meshapp/engine-api";

export const LAB_THRESHOLDS = {
  version: 1,
  T0: { maxLoss: 0.05, owdP95Ms: 60, minStreams: 2, bleFastKbps: 150 },
  T0b: { maxLoss: 0.05, hopP95Ms: 40 },
  T0c: { maxLoss: 0.05, owdP95Ms: 60, maxXruns: 0 },
  T1: { maxLoss: 0.05, windowOkShare: 0.95, minWindows: 60, convoyGapM: 50, distanceBinM: 10 },
  T2: { maxSilentMutes: 0, minDurationS: 60 },
  T3: { rejoinS: 10 },
  T4: { streams: 4, maxLoss: 0.02 },
  T5a: { legMs: 25 },
  T6: { batteryPctPerH: 5, minDurationS: 1800 },
  T8: { touchToFlagP95Ms: 50, maxFrameMs: 50 },
  voxCal: { falseTriggersPerMin: 1, onsetMs: 150 },
} as const;

const pct = (ratio: number): number => Math.round(ratio * 1000) / 10;

/**
 * Interpolation params of the `bench.<id>.criterion` text (lab catalog), so the copy
 * always shows the current thresholds. T5b has no numeric threshold (a per-model note).
 */
export function criterionParams(bench: LabBench): Record<string, number> {
  const t = LAB_THRESHOLDS;
  switch (bench) {
    case "T0":
      return {
        streams: t.T0.minStreams,
        lossPct: pct(t.T0.maxLoss),
        owdMs: t.T0.owdP95Ms,
        fastKbps: t.T0.bleFastKbps,
      };
    case "T0b":
      return { lossPct: pct(t.T0b.maxLoss), hopMs: t.T0b.hopP95Ms };
    case "T0c":
      return { lossPct: pct(t.T0c.maxLoss), owdMs: t.T0c.owdP95Ms, xruns: t.T0c.maxXruns };
    case "T1":
      return { sharePct: pct(t.T1.windowOkShare), lossPct: pct(t.T1.maxLoss), gapM: t.T1.convoyGapM };
    case "T2":
      return { mutes: t.T2.maxSilentMutes };
    case "T3":
      return { rejoinS: t.T3.rejoinS };
    case "T4":
      return { streams: t.T4.streams, lossPct: pct(t.T4.maxLoss) };
    case "T5a":
      return { legMs: t.T5a.legMs };
    case "T6":
      return { pctPerH: t.T6.batteryPctPerH, minMin: t.T6.minDurationS / 60 };
    case "T8":
      return { p95Ms: t.T8.touchToFlagP95Ms, frameMs: t.T8.maxFrameMs };
    case "voxCal":
      return { perMin: t.voxCal.falseTriggersPerMin, onsetMs: t.voxCal.onsetMs };
    case "T5b":
    case "ride":
      return {};
  }
}
