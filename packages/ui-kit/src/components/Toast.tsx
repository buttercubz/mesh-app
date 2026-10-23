import { useSyncExternalStore } from "react";
import { cx } from "../cx.js";
import { toaster as defaultToaster, type ToastStore, type ToastTone } from "./toast-store.js";

export interface ToastProps {
  readonly title: string;
  readonly description?: string;
  readonly tone?: ToastTone;
  /** Accessible name of the dismiss button (translated by the app). */
  readonly dismissLabel: string;
  readonly onDismiss: () => void;
  readonly className?: string;
}

/** One transient notice (glass card, 44 px dismiss target). */
export function Toast({ title, description, tone = "neutral", dismissLabel, onDismiss, className }: ToastProps) {
  return (
    <div className={cx("mk-toast", "glass", className)} data-tone={tone} role={tone === "danger" ? "alert" : "status"}>
      <span className="mk-toast-text">
        <span className="mk-toast-title">{title}</span>
        {description !== undefined && <span className="mk-toast-description">{description}</span>}
      </span>
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
}

/** Renders the toast queue at the top of the screen (below the safe area). */
export function Toaster({ dismissLabel, label, toaster = defaultToaster, className }: ToasterProps) {
  const items = useSyncExternalStore(toaster.subscribe, toaster.list, toaster.list);
  return (
    <ol aria-label={label} className={cx("mk-toast-viewport", className)}>
      {items.map((item) => (
        <li key={item.id}>
          <Toast
            title={item.title}
            description={item.description}
            tone={item.tone}
            dismissLabel={dismissLabel}
            onDismiss={() => toaster.dismiss(item.id)}
          />
        </li>
      ))}
    </ol>
  );
}
