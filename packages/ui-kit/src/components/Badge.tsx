import type { HTMLAttributes } from "react";
import { cx } from "../cx.js";

/** Badge tones map 1:1 to the intercom tokens. */
export type BadgeTone = "neutral" | "brand" | "active" | "blocked" | "relayed" | "out" | "warn" | "danger";

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  readonly tone?: BadgeTone;
}

/** Small non-interactive capsule (status, "unconfirmed", "best effort", hops...). */
export function Badge({ tone = "neutral", className, ...rest }: BadgeProps) {
  return (
    <span
      data-tone={tone}
      className={cx("mk-badge", tone !== "neutral" && `mk-badge--${tone}`, className)}
      {...rest}
    />
  );
}
