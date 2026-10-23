// Live-view data helpers (WP-21, DESIGN §12.5): the 60 s sparkline windows built from the
// bounded labProgress ring of useLabStore (2 Hz), and the metrics the big numbers show.
import type { LabLiveSample } from "@meshapp/engine-api";

import type { LabSamplePoint } from "../../store/useLabStore";

/** Sparkline window, seconds. */
export const SPARK_WINDOW_S = 60;

export type LiveMetric = "txKbps" | "loss" | "rttP95Ms" | "rssi" | "distM";

/** Metrics with a 60 s sparkline, in display order. */
export const SPARK_METRICS: readonly LiveMetric[] = ["txKbps", "loss", "rttP95Ms", "rssi", "distM"];

/** The value of one metric in a sample (`null` = no data, a gap in the line). */
export function metricValue(sample: LabLiveSample, metric: LiveMetric): number | null {
  const v = sample[metric];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** Samples of the last `windowS` seconds (relative to the newest sample). */
export function lastWindow(
  samples: readonly LabSamplePoint[],
  windowS: number = SPARK_WINDOW_S,
): readonly LabSamplePoint[] {
  const newest = samples[samples.length - 1];
  if (newest === undefined) return [];
  const from = newest.elapsedS - windowS;
  let i = samples.length - 1;
  while (i > 0 && (samples[i - 1]?.elapsedS ?? -Infinity) > from) i -= 1;
  return samples.slice(i);
}

/** One metric over the last 60 s, oldest first. */
export function series(samples: readonly LabSamplePoint[], metric: LiveMetric): (number | null)[] {
  return lastWindow(samples).map((p) => metricValue(p.sample, metric));
}

/** `true` when the samples show the anchor lost at least once in the window. */
export function anchorDropped(samples: readonly LabSamplePoint[]): boolean {
  return lastWindow(samples).some((p) => !p.sample.anchorOk);
}

/** `mm:ss`, or `h:mm:ss` from one hour. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(sec)}` : `${two(m)}:${two(sec)}`;
}
