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
  readonly className?: string;
}

/** Persistent in-page banner. Urgent tones are announced assertively. */
export function Banner({ tone = "info", title, message, icon, action, className }: BannerProps) {
  const urgent = tone === "danger" || tone === "blocked";
  return (
    <div role={urgent ? "alert" : "status"} data-tone={tone} className={cx("mk-banner", className)}>
      {icon !== undefined && (
        <span className="mk-banner-icon" aria-hidden="true">
          {icon}
        </span>
      )}
      <span className="mk-banner-text">
        <span className="mk-banner-title">{title}</span>
        {message !== undefined && <span className="mk-banner-message">{message}</span>}
      </span>
      {action && (
        <Button variant="secondary" onClick={action.onClick}>
          {action.label}
        </Button>
      )}
    </div>
  );
}
