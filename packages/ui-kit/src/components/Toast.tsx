import type { ReactNode } from "react";
import { useState, useSyncExternalStore } from "react";
import { cx } from "../cx.js";
import { toaster as defaultToaster, type ToastItem, type ToastStore, type ToastTone } from "./toast-store.js";

export interface ToastProps {
  readonly title: string;
  readonly description?: string;
  readonly tone?: ToastTone;
  /** Accessible name of the dismiss button (translated by the app). */
  readonly dismissLabel: string;
  readonly onDismiss: () => void;
  readonly className?: string;
  /** Extra control before the dismiss button (the Toaster's "+n" for queued toasts). */
  readonly extra?: ReactNode;
}

/** One transient notice (glass card, 44 px dismiss target). */
export function Toast({ title, description, tone = "neutral", dismissLabel, onDismiss, className, extra }: ToastProps) {
  return (
    <div className={cx("mk-toast", "glass", className)} data-tone={tone} role={tone === "danger" ? "alert" : "status"}>
      <span className="mk-toast-text">
        <span className="mk-toast-title">{title}</span>
        {description !== undefined && <span className="mk-toast-description">{description}</span>}
      </span>
      {extra}
      <button type="button" className="mk-toast-close" aria-label={dismissLabel} onClick={onDismiss}>
        <span aria-hidden="true">✕</span>
      </button>
    </div>
  );
}

export interface ToasterProps {
  /** Accessible name of every dismiss button (translated by the app). */
  readonly dismissLabel: string;
  /** Accessible name of the region (e.g. "Notifications"). */
  readonly label: string;
  readonly toaster?: ToastStore;
  readonly className?: string;
  /**
   * Show at most this many (the newest); the rest wait behind a "+n" button in the top
   * visible toast that expands them, so a stack of notices never covers the screen.
   * Default: all.
   */
  readonly maxVisible?: number;
  /** Accessible name of the "+n" button (translated by the app), e.g. n => `${n} more`. */
  readonly moreLabel?: (hidden: number) => string;
  /** Items to leave out of the viewport (e.g. a notice already shown as an on-screen banner). */
  readonly filter?: (item: ToastItem) => boolean;
}

/**
 * Renders the toast queue (top of the screen below the safe area by default; the app may
 * dock it elsewhere with `className`). The newest toast is last, next to the dock edge.
 */
export function Toaster({ dismissLabel, label, toaster = defaultToaster, className, maxVisible, moreLabel, filter }: ToasterProps) {
  const all = useSyncExternalStore(toaster.subscribe, toaster.list, toaster.list);
  // Expanded for the queue as it was when the pill was tapped; a newer toast collapses it again.
  const [expandedFor, setExpandedFor] = useState<number | null>(null);
  const items = filter === undefined ? all : all.filter(filter);
  const newest = items[items.length - 1]?.id ?? null;
  const showAll = maxVisible === undefined || (expandedFor !== null && expandedFor === newest);
  const limit = showAll ? items.length : Math.max(1, maxVisible);
  const hidden = Math.max(0, items.length - limit);
  const shown = items.slice(items.length - Math.min(limit, items.length));
  return (
    <ol aria-label={label} className={cx("mk-toast-viewport", className)}>
      {shown.map((item, index) => (
        <li key={item.id}>
          <Toast
            title={item.title}
            description={item.description}
            tone={item.tone}
            dismissLabel={dismissLabel}
            onDismiss={() => toaster.dismiss(item.id)}
            extra={
              index === 0 && hidden > 0 ? (
                <button
                  type="button"
                  className="mk-toast-more"
                  aria-label={moreLabel ? moreLabel(hidden) : `+${hidden}`}
                  aria-expanded={false}
                  onClick={() => setExpandedFor(newest)}
                >
                  <span aria-hidden="true">+{hidden}</span>
                </button>
              ) : undefined
            }
          />
        </li>
      ))}
    </ol>
  );
}
