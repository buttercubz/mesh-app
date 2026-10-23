import { useEffect, useId, useRef } from "react";
import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import { cx } from "../cx.js";

export type SheetCloseReason = "scrim" | "escape" | "button";

export interface SheetProps {
  readonly open: boolean;
  readonly onClose: (reason: SheetCloseReason) => void;
  readonly title: ReactNode;
  /** Accessible name of the close button (translated by the app, e.g. "Close"). */
  readonly closeLabel: string;
  readonly children: ReactNode;
  readonly className?: string;
  /** Portal target; defaults to document.body. */
  readonly container?: HTMLElement | null;
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Modal bottom sheet: glass-strong panel over a scrim, Escape and scrim tap close it,
 * focus moves in on open, stays inside (Tab wraps) and returns on close; the page
 * behind does not scroll. Controlled by `open`.
 */
export function Sheet({ open, onClose, title, closeLabel, children, className, container }: SheetProps) {
  const panel = useRef<HTMLDivElement>(null);
  const titleId = useId();

  useEffect(() => {
    if (!open) return;
    const previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const body = document.body;
    const previousOverflow = body.style.overflow;
    body.style.overflow = "hidden";
    panel.current?.focus();
    return () => {
      body.style.overflow = previousOverflow;
      previouslyFocused?.focus();
    };
  }, [open]);

  if (!open) return null;
  const target = container ?? (typeof document === "undefined" ? null : document.body);
  if (!target) return null;

  const onScrimClick = (e: MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) onClose("scrim");
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose("escape");
      return;
    }
    if (e.key !== "Tab" || !panel.current) return;
    const items = Array.from(panel.current.querySelectorAll<HTMLElement>(FOCUSABLE));
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) {
      e.preventDefault();
      return;
    }
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === panel.current)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div className="mk-sheet-scrim" data-state="open" onClick={onScrimClick}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={cx("mk-sheet", "glass-strong", className)}
        data-state="open"
        onKeyDown={onKeyDown}
      >
        <div className="mk-sheet-handle" aria-hidden="true" />
        <div className="mk-sheet-header">
          <h2 id={titleId} className="mk-sheet-title">
            {title}
          </h2>
          <button
            type="button"
            className="mk-button mk-button--secondary mk-button--icon"
            aria-label={closeLabel}
            onClick={() => onClose("button")}
          >
            <span aria-hidden="true">✕</span>
          </button>
        </div>
        <div className="mk-sheet-body">{children}</div>
      </div>
    </div>,
    target,
  );
}
