import { cx } from "../cx.js";

export type QualityTone = "none" | "poor" | "fair" | "good";

/** Rounds and clamps `bars` into 0..max (NaN reads as 0). */
export function clampBars(bars: number, max: number): number {
  const top = Math.max(1, Math.floor(max));
  if (!Number.isFinite(bars) || bars <= 0) return 0;
  return Math.min(top, Math.round(bars));
}

/**
 * Tone for a bar count: 0 → none (status-out), up to a third → poor (danger),
 * up to two thirds → fair (ladder-warn), above → good (talk-active).
 */
export function qualityTone(bars: number, max = 4): QualityTone {
  const top = Math.max(1, Math.floor(max));
  const n = clampBars(bars, top);
  if (n === 0) return "none";
  const ratio = n / top;
  if (ratio <= 1 / 3) return "poor";
  if (ratio <= 2 / 3) return "fair";
  return "good";
}

export interface QualityBarsProps {
  /** Filled bars, 0..max (the app maps link quality to bars). */
  readonly bars: number;
  readonly max?: number;
  /** Accessible description, e.g. "Link quality: good" (translated by the app). */
  readonly label: string;
  readonly className?: string;
}

/** Signal-style link-quality meter. */
export function QualityBars({ bars, max = 4, label, className }: QualityBarsProps) {
  const top = Math.max(1, Math.floor(max));
  const filled = clampBars(bars, top);
  const items = Array.from({ length: top }, (_, i) => i);
  return (
    <span
      role="img"
      aria-label={label}
      className={cx("mk-quality", className)}
      data-tone={qualityTone(filled, top)}
      data-bars={filled}
    >
      {items.map((i) => (
        <span
          key={i}
          className="mk-quality-bar"
          data-filled={i < filled ? "true" : "false"}
          style={{ height: `${Math.round(((i + 1) / top) * 100)}%` }}
        />
      ))}
    </span>
  );
}
