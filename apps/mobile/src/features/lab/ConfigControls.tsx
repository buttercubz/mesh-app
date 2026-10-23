// Controls of the bench config sheet (WP-21, DESIGN §12.5): variant axes, the mandatory
// audio path and keep-alive anchor (the bench default pre-selected; `none` labelled as
// the suspension control) and the per-run GPS consent.
import type { LabAnchor, LabAudio } from "@meshapp/engine-api";
import { LAB_AUDIOS } from "@meshapp/engine-api";
import { SegmentedControl } from "@meshapp/ui-kit";
import { useId } from "react";

import { useT } from "../../i18n/index";
import type { BenchAxis, BenchSpec } from "./catalog";

/** Axes with more values than this use a picker instead of a segmented control. */
const SEGMENT_MAX = 3;

export function AxisControl({
  axis,
  value,
  onChange,
}: {
  readonly axis: BenchAxis;
  readonly value: string;
  readonly onChange: (value: string) => void;
}) {
  const { t } = useT("lab");
  const id = useId();
  const label = t(`axis.${axis.key}`);
  if (axis.values.length <= SEGMENT_MAX) {
    return (
      <div className="flex flex-col gap-1">
        <span className="text-sm font-medium" aria-hidden="true">
          {label}
        </span>
        <SegmentedControl
          label={label}
          value={value}
          onChange={onChange}
          options={axis.values.map((v) => ({ value: v, label: t(`axis.${axis.key}.${v}`) }))}
        />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-sm font-medium">
        {label}
      </label>
      <select
        id={id}
        className="min-h-target rounded-xl bg-control-fill px-3"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {axis.values.map((v) => (
          <option key={v} value={v}>
            {t(`axis.${axis.key}.${v}`)}
          </option>
        ))}
      </select>
    </div>
  );
}

/** Audio path picker (mandatory). */
export function AudioControl({
  value,
  onChange,
}: {
  readonly value: LabAudio | null;
  readonly onChange: (value: LabAudio) => void;
}) {
  const { t } = useT("lab");
  return (
    <fieldset className="flex flex-col gap-1" data-testid="lab-audio">
      <legend className="text-sm font-medium">{t("config.audio")}</legend>
      <SegmentedControl<LabAudio | "">
        label={t("config.audio")}
        value={value ?? ""}
        onChange={(v) => {
          if (v !== "") onChange(v);
        }}
        options={LAB_AUDIOS.map((a) => ({ value: a, label: t(`audio.${a}`) }))}
      />
      <p className="text-xs text-muted-foreground">
        {value === null ? t("config.audioHint") : `${t(`audio.${value}.hint`)} ${t("config.audioHint")}`}
      </p>
    </fieldset>
  );
}

/** Keep-alive anchor picker (mandatory), limited to the anchors the bench uses. */
export function AnchorControl({
  spec,
  value,
  onChange,
}: {
  readonly spec: BenchSpec;
  readonly value: LabAnchor | null;
  readonly onChange: (value: LabAnchor) => void;
}) {
  const { t } = useT("lab");
  return (
    <fieldset className="flex flex-col gap-1" data-testid="lab-anchor">
      <legend className="text-sm font-medium">{t("config.anchor")}</legend>
      <SegmentedControl<LabAnchor | "">
        label={t("config.anchor")}
        value={value ?? ""}
        onChange={(v) => {
          if (v !== "") onChange(v);
        }}
        options={spec.anchors.map((a) => ({ value: a, label: t(`anchor.${a}`) }))}
      />
      <p className="text-xs text-muted-foreground" data-anchor-hint={value ?? ""}>
        {value === null ? t("config.anchorHint") : `${t(`anchor.${value}.hint`)} ${t("config.anchorHint")}`}
      </p>
    </fieldset>
  );
}

/** Per-run GPS consent. */
export function GpsControl({
  checked,
  onChange,
}: {
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
}) {
  const { t } = useT("lab");
  const id = useId();
  return (
    <div className="flex items-start gap-3">
      <input
        id={id}
        type="checkbox"
        className="mt-1 size-6 shrink-0 accent-brand"
        checked={checked}
        aria-describedby={`${id}-hint`}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="flex flex-col">
        <label htmlFor={id} className="min-h-6 text-sm font-medium">
          {t("config.gps")}
        </label>
        <span id={`${id}-hint`} className="text-xs text-muted-foreground">
          {t("config.gpsHint")}
        </span>
      </span>
    </div>
  );
}
