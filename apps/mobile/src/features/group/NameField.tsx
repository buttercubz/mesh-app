// Display-name field (WP-22, DESIGN §1.3 F0): the name other riders see and hear in the
// prompts, limited to 24 UTF-8 bytes and cut only at a grapheme boundary, with a live
// byte counter so accented letters and emoji never surprise the rider.
import { DISPLAY_NAME_MAX_BYTES } from "@meshapp/engine-api";
import { useId } from "react";

import { useT } from "../../i18n/index";
import { clampDisplayName } from "../../store/useSettingsStore";
import { byteLength, limitName } from "./name";

export interface NameFieldProps {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly autoFocus?: boolean;
}

export function NameField({ value, onChange, autoFocus }: NameFieldProps) {
  const { t } = useT("group");
  const id = useId();
  const hintId = useId();
  const used = byteLength(clampDisplayName(value));
  return (
    <div className="flex flex-col gap-2">
      <label htmlFor={id} className="text-sm font-semibold">
        {t("name.label")}
      </label>
      <input
        id={id}
        className="min-h-14 rounded-2xl border border-separator bg-card px-4 text-lg text-card-foreground shadow-surface outline-none transition-shadow duration-ui focus:border-brand focus:ring-4 focus:ring-brand/20"
        value={value}
        autoComplete="nickname"
        autoCapitalize="words"
        // Only the onboarding name step focuses the field on open.
        autoFocus={autoFocus}
        aria-describedby={hintId}
        placeholder={t("name.placeholder")}
        onChange={(e) => onChange(limitName(e.target.value))}
      />
      <p id={hintId} className="flex justify-between gap-2 text-xs text-muted-foreground">
        <span>{t("name.hint")}</span>
        <span className="tabular-nums" aria-label={t("name.bytes", { n: used, max: DISPLAY_NAME_MAX_BYTES })}>
          {used}/{DISPLAY_NAME_MAX_BYTES}
        </span>
      </p>
    </div>
  );
}
