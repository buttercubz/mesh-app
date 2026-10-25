import type { ReactNode } from "react";
import { cx } from "../cx.js";

export interface ListRowProps {
  readonly title: ReactNode;
  readonly subtitle?: ReactNode;
  /** Icon, avatar or status dot on the leading edge. */
  readonly leading?: ReactNode;
  /** Value, badge, quality bars or chevron on the trailing edge. */
  readonly trailing?: ReactNode;
  /** Makes the row a 48 px+ button. */
  readonly onClick?: () => void;
  readonly disabled?: boolean;
  readonly className?: string;
  /** Accessible name of the interactive row when the visible text is not enough. */
  readonly ariaLabel?: string;
}

/** Settings / members row (convLink .ios-list-row). Interactive when `onClick` is set. */
export function ListRow({ title, subtitle, leading, trailing, onClick, disabled, className, ariaLabel }: ListRowProps) {
  const content = (
    <>
      {leading !== undefined && <span className="mk-list-row-leading">{leading}</span>}
      <span className="mk-list-row-text">
        <span className="mk-list-row-title">{title}</span>
        {subtitle !== undefined && <span className="mk-list-row-subtitle">{subtitle}</span>}
      </span>
      {trailing !== undefined && <span className="mk-list-row-trailing">{trailing}</span>}
    </>
  );
  if (onClick) {
    return (
      <button
        type="button"
        className={cx("mk-list-row", "mk-list-row--interactive", className)}
        onClick={onClick}
        disabled={disabled}
        aria-label={ariaLabel}
      >
        {content}
      </button>
    );
  }
  return (
    <div className={cx("mk-list-row", className)}>
      {content}
    </div>
  );
}

export interface ListSectionProps {
  /** Section heading shown above the grouped rows. */
  readonly title?: ReactNode;
  /** One ListRow per entry. */
  readonly children: ReactNode;
  readonly className?: string;
}

/** Grouped inset section (convLink .ios-section) holding ListRows. */
export function ListSection({ title, children, className }: ListSectionProps) {
  return (
    <section className={className}>
      {title !== undefined && <h3 className="mk-list-section-title">{title}</h3>}
      <div className="mk-list-section">{children}</div>
    </section>
  );
}
