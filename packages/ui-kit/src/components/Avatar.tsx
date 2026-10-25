import type { CSSProperties } from "react";
import { cx } from "../cx.js";
import { clampLevel } from "./ptt-gesture.js";

/** Status dot tones (colour is always paired with text elsewhere on the screen). */
export type AvatarDotTone = "active" | "relayed" | "warn" | "danger" | "brand" | "out";

export interface AvatarProps {
  /** Rider name: gives the initials and a stable per-rider hue. */
  readonly name: string;
  /** Diameter in px (default 44). */
  readonly size?: number;
  /** Live voice level 0..1: draws a ring around the avatar when set. */
  readonly level?: number;
  /** Pulsing halo while this rider is talking (stops under reduced motion). */
  readonly speaking?: boolean;
  /** Small status dot at the bottom-right. */
  readonly dot?: AvatarDotTone;
  /** Greyed out (lost / out of range). */
  readonly dim?: boolean;
  readonly className?: string;
}

const RING_R = 48;
const RING_C = 2 * Math.PI * RING_R;

/** Up to two initials, grapheme-aware ("Ana María" → "AM", "Ñandú" → "Ñ"). */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter((w) => w !== "");
  const first = (w: string | undefined) => (w === undefined ? "" : (Array.from(w)[0] ?? "").toLocaleUpperCase());
  if (words.length === 0) return "?";
  if (words.length === 1) return first(words[0]);
  return first(words[0]) + first(words[words.length - 1]);
}

/** Stable hue 0..359 for a name (FNV-1a), so a rider keeps one colour everywhere. */
export function avatarHue(name: string): number {
  let h = 0x811c9dc5;
  for (const ch of name.trim().toLocaleLowerCase()) {
    h ^= ch.codePointAt(0) ?? 0;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h % 360;
}

/** Decorative rider avatar: initials on a per-rider colour. Hidden from screen readers. */
export function Avatar({ name, size = 44, level, speaking = false, dot, dim = false, className }: AvatarProps) {
  const style = { "--mk-avatar-size": `${size}px`, "--mk-avatar-hue": String(avatarHue(name)) } as CSSProperties;
  return (
    <span
      className={cx("mk-avatar", className)}
      style={style}
      aria-hidden="true"
      data-dim={dim ? "true" : undefined}
      data-speaking={speaking ? "true" : undefined}
    >
      {level !== undefined && (
        <svg className="mk-avatar-ring" viewBox="0 0 100 100" focusable="false">
          <circle className="mk-avatar-ring-track" cx="50" cy="50" r={RING_R} strokeWidth="4" />
          <circle
            className="mk-avatar-ring-level"
            cx="50"
            cy="50"
            r={RING_R}
            strokeWidth="4"
            strokeDasharray={RING_C}
            strokeDashoffset={RING_C * (1 - clampLevel(level))}
          />
        </svg>
      )}
      {initials(name)}
      {dot !== undefined && <span className="mk-avatar-dot" data-tone={dot} />}
    </span>
  );
}
