// Settings building blocks (WP-22): a labelled row holding one control, and an
// accessible on/off switch (role="switch", 44 px target, state in text for screen
// readers and colour for sighted riders).
import { cx } from "@meshapp/ui-kit";
import type { ReactNode } from "react";

export interface SettingRowProps {
  readonly label: ReactNode;
  readonly description?: ReactNode;
  /** The control; rendered beside the label, or under it when `stacked`. */
  readonly children: ReactNode;
  readonly stacked?: boolean;
  readonly testId?: string;
}

export function SettingRow({ label, description, children, stacked = false, testId }: SettingRowProps) {
  return (
    <div
      className={cx("flex gap-3 px-4 py-3", stacked ? "flex-col" : "items-center justify-between")}
      data-testid={testId}
    >
      <div className="min-w-0 flex-1">
        <div className="text-base font-medium">{label}</div>
        {description !== undefined && <div className="mt-0.5 text-sm text-muted-foreground">{description}</div>}
      </div>
      <div className={stacked ? "w-full" : "shrink-0"}>{children}</div>
    </div>
  );
}

export interface SwitchProps {
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
  readonly label: string;
  readonly disabled?: boolean;
}

export function Switch({ checked, onChange, label, disabled = false }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="grid min-h-target min-w-target place-items-center disabled:opacity-50"
    >
      <span
        aria-hidden="true"
        className={cx(
          "relative h-8 w-13 rounded-full transition-colors duration-ui",
          checked ? "bg-talk-active" : "bg-control-fill ring-1 ring-muted-foreground ring-inset",
        )}
      >
        <span
          className={cx(
            "absolute top-1 size-6 rounded-full bg-white shadow-surface transition-[left] duration-ui",
            checked ? "left-6" : "left-1",
          )}
        />
      </span>
    </button>
  );
}

/** Grouped card of setting rows with a heading. */
export function SettingsGroup({ title, children }: { readonly title: string; readonly children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h2 className="px-4 text-xs font-semibold tracking-wide text-muted-foreground uppercase">{title}</h2>
      <div className="divide-y divide-separator rounded-section bg-card text-card-foreground">{children}</div>
    </section>
  );
}
