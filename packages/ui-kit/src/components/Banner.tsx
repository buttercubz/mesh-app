import type { ReactNode } from "react";
import { cx } from "../cx.js";
import { Button } from "./Button.js";

/**
 * info: island / alone (relayed blue) · warn: degradation ladder L1-L3 ·
 * blocked: channel full (amber) · danger: Bluetooth off, capture lost · neutral.
 */
export type BannerTone = "info" | "warn" | "blocked" | "danger" | "neutral";

export interface BannerAction {
  readonly label: string;
  readonly onClick: () => void;
}

export interface BannerProps {
  readonly tone?: BannerTone;
  readonly title: ReactNode;
  readonly message?: ReactNode;
  readonly icon?: ReactNode;
  /** Optional single action, e.g. "Resume" for the ladder banner. */
  readonly action?: BannerAction;
  /**
   * `full` (default): title and message. `bar`: one compact row for glance screens (title
   * clamped to two lines, no message; pair it with `onExpand` so the rest stays reachable).
   */
  readonly variant?: "full" | "bar";
  /** Makes the text a button that opens the details (e.g. a sheet with the message). */
  readonly onExpand?: () => void;
  /** Whether the details opened by `onExpand` are showing (aria-expanded). */
  readonly expanded?: boolean;
  /** Id of the element `onExpand` shows (aria-controls). */
  readonly controls?: string;
  readonly className?: string;
}

/** Persistent in-page banner. Urgent tones are announced assertively. */
export function Banner({
  tone = "info",
  title,
  message,
  icon,
  action,
  variant = "full",
  onExpand,
  expanded,
  controls,
  className,
}: BannerProps) {
  const urgent = tone === "danger" || tone === "blocked";
  const text = (
    <>
      <span className="mk-banner-title">{title}</span>
      {message !== undefined && variant === "full" && <span className="mk-banner-message">{message}</span>}
    </>
  );
  return (
    <div
      role={urgent ? "alert" : "status"}
      data-tone={tone}
      className={cx("mk-banner", variant === "bar" && "mk-banner--bar", className)}
    >
      {icon !== undefined && (
        <span className="mk-banner-icon" aria-hidden="true">
          {icon}
        </span>
      )}
      {onExpand !== undefined ? (
        <button
          type="button"
          className="mk-banner-text"
          aria-haspopup="dialog"
          aria-expanded={expanded}
          aria-controls={controls}
          onClick={onExpand}
        >
          {text}
        </button>
      ) : (
        <span className="mk-banner-text">{text}</span>
      )}
      {action && (
        <Button variant="secondary" onClick={action.onClick}>
          {action.label}
        </Button>
      )}
    </div>
  );
}
