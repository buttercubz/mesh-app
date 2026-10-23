import { useRef } from "react";
import type { KeyboardEvent, ReactNode } from "react";
import { cx } from "../cx.js";

export interface SegmentOption<T extends string> {
  readonly value: T;
  readonly label: ReactNode;
  readonly icon?: ReactNode;
  readonly disabled?: boolean;
  /** Accessible name when `label` is not plain text. */
  readonly ariaLabel?: string;
}

export interface SegmentedControlProps<T extends string> {
  readonly options: ReadonlyArray<SegmentOption<T>>;
  readonly value: T;
  readonly onChange: (value: T) => void;
  /** Accessible name of the group (e.g. "Talk mode"), translated by the app. */
  readonly label: string;
  readonly disabled?: boolean;
  readonly className?: string;
}

/**
 * Next enabled index from `from` moving by `step` (wrapping), or `from` when no other
 * option is enabled. Exported for tests.
 */
export function nextEnabledIndex(enabled: ReadonlyArray<boolean>, from: number, step: 1 | -1): number {
  const n = enabled.length;
  for (let k = 1; k <= n; k++) {
    const i = (((from + step * k) % n) + n) % n;
    if (enabled[i]) return i;
  }
  return from;
}

/**
 * iOS-style segmented control (convLink .ios-segmented) with radio-group semantics:
 * one tab stop, arrow keys / Home / End move and select, 44 px segments.
 */
export function SegmentedControl<T extends string>(props: SegmentedControlProps<T>) {
  const { options, value, onChange, label, disabled = false, className } = props;
  const group = useRef<HTMLDivElement>(null);
  const enabled = options.map((o) => !disabled && !o.disabled);
  const selected = options.findIndex((o) => o.value === value);
  const firstEnabled = enabled.indexOf(true);
  const tabStop = selected >= 0 && enabled[selected] ? selected : firstEnabled;

  const select = (index: number) => {
    const option = options[index];
    if (!option || !enabled[index]) return;
    const buttons = group.current?.querySelectorAll<HTMLButtonElement>('[role="radio"]');
    buttons?.[index]?.focus();
    if (option.value !== value) onChange(option.value);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const current = selected >= 0 ? selected : tabStop;
    let target: number | null = null;
    switch (e.key) {
      case "ArrowRight":
      case "ArrowDown":
        target = nextEnabledIndex(enabled, current, 1);
        break;
      case "ArrowLeft":
      case "ArrowUp":
        target = nextEnabledIndex(enabled, current, -1);
        break;
      case "Home":
        target = firstEnabled;
        break;
      case "End":
        target = enabled.lastIndexOf(true);
        break;
      default:
        return;
    }
    e.preventDefault();
    if (target >= 0) select(target);
  };

  return (
    <div
      ref={group}
      role="radiogroup"
      aria-label={label}
      aria-disabled={disabled || undefined}
      className={cx("mk-segmented", className)}
      onKeyDown={onKeyDown}
    >
      {options.map((option, index) => {
        const checked = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={checked}
            aria-label={option.ariaLabel}
            tabIndex={index === tabStop ? 0 : -1}
            disabled={!enabled[index]}
            className="mk-segment"
            data-value={option.value}
            onClick={() => select(index)}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
