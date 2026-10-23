import { cx } from "../cx.js";

/** A sample; `null` / NaN is a gap (no data for that slot). */
export type SparkValue = number | null;

export interface SparkDomain {
  readonly min: number;
  readonly max: number;
}

/** Domain of the finite values, widened by `min`/`max` overrides; flat data gets ±1. */
export function sparkDomain(values: ReadonlyArray<SparkValue>, min?: number, max?: number): SparkDomain {
  let lo = Number.POSITIVE_INFINITY;
  let hi = Number.NEGATIVE_INFINITY;
  for (const v of values) {
    if (v === null || !Number.isFinite(v)) continue;
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  if (min !== undefined && Number.isFinite(min)) lo = min;
  if (max !== undefined && Number.isFinite(max)) hi = max;
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) return { min: 0, max: 1 };
  if (hi === lo) return { min: lo - 1, max: hi + 1 };
  return lo < hi ? { min: lo, max: hi } : { min: hi, max: lo };
}

const round = (n: number) => Math.round(n * 100) / 100;

/**
 * SVG path for the samples laid out evenly over `width`, value axis inverted to fit
 * `height`. Gaps break the line (a new `M` segment); values outside the domain are
 * clamped to the edges. Returns "" when there is nothing to draw.
 */
export function sparklinePath(
  values: ReadonlyArray<SparkValue>,
  width: number,
  height: number,
  domain: SparkDomain = sparkDomain(values),
): string {
  const n = values.length;
  if (n === 0 || width <= 0 || height <= 0) return "";
  const step = n === 1 ? 0 : width / (n - 1);
  const span = domain.max - domain.min || 1;
  const segments: string[][] = [];
  let segment: string[] | null = null;
  for (let i = 0; i < n; i++) {
    const v = values[i];
    if (v === null || v === undefined || !Number.isFinite(v)) {
      segment = null; // a gap ends the current segment
      continue;
    }
    const t = Math.min(1, Math.max(0, (v - domain.min) / span));
    const x = round(n === 1 ? width / 2 : i * step);
    const y = round(height - t * height);
    if (!segment) {
      segment = [];
      segments.push(segment);
    }
    segment.push(`${x} ${y}`);
  }
  return segments.map((points) => `M${points.join(" L")}`).join(" ");
}

export interface SparklineProps {
  readonly values: ReadonlyArray<SparkValue>;
  /** Accessible summary (e.g. "Loss, last 60 s, now 3 %"), translated by the app. */
  readonly label: string;
  readonly width?: number;
  readonly height?: number;
  readonly min?: number;
  readonly max?: number;
  /** Optional dashed reference line (e.g. a go/no-go threshold). */
  readonly threshold?: number;
  readonly className?: string;
}

/** Small trend line for the Field test live view (60 s windows, DESIGN §12.5). */
export function Sparkline({ values, label, width = 120, height = 32, min, max, threshold, className }: SparklineProps) {
  const domain = sparkDomain(threshold === undefined ? values : [...values, threshold], min, max);
  const d = sparklinePath(values, width, height, domain);
  const thresholdY =
    threshold === undefined || !Number.isFinite(threshold)
      ? null
      : round(height - Math.min(1, Math.max(0, (threshold - domain.min) / (domain.max - domain.min || 1))) * height);
  return (
    <svg
      role="img"
      aria-label={label}
      className={cx("mk-sparkline", className)}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
    >
      {thresholdY !== null && (
        <line className="mk-sparkline-threshold" x1={0} x2={width} y1={thresholdY} y2={thresholdY} />
      )}
      {d && <path className="mk-sparkline-line" d={d} />}
    </svg>
  );
}
