// Live input level of an audio check (WP-22, DESIGN §1.3 F0, §11.4): the bar follows
// `audioCheckSample.level`, its colour and text follow the VAD gate, and the detected
// route and input rate are spelled out ("Headset · HD voice · 16 kHz").
import type { AudioCheckPhase } from "@meshapp/engine-api";

import { useT } from "../../i18n/index";
import type { AudioCheckReading } from "../../store/useEngineStore";
import { routeKey } from "../ride/model";

export interface LevelMeterProps {
  readonly reading: AudioCheckReading | null;
  /** Label of the meter (accessible name). */
  readonly label: string;
}

/** Live input level bar with the VAD gate state, from `audioCheckSample`. */
export function LevelMeter({ reading, label }: LevelMeterProps) {
  const { t } = useT("onboarding");
  const { t: tr } = useT("ride");
  const level = Math.round(Math.min(1, Math.max(0, reading?.level ?? 0)) * 100);
  const open = reading?.gate === "open";
  const phase: AudioCheckPhase | null = reading?.phase ?? null;
  return (
    <div className="flex flex-col gap-2" data-testid="level-meter" data-gate={reading?.gate ?? "none"}>
      <div
        className="h-4 w-full overflow-hidden rounded-full bg-control-fill"
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={level}
      >
        <div
          className={`h-full rounded-full transition-[width] duration-100 ${open ? "bg-talk-active" : "bg-status-relayed"}`}
          style={{ width: `${level}%` }}
        />
      </div>
      <div className="flex flex-wrap justify-between gap-2 text-sm">
        <span data-testid="meter-gate">{reading === null ? t("check.waiting") : open ? t("check.voice") : t("check.quiet")}</span>
        {reading !== null && (
          <span className="text-muted-foreground" data-testid="meter-route">
            {t("check.routeLine", { route: tr(routeKey(reading.route)), rate: Math.round(reading.inputRate / 1000) })}
          </span>
        )}
      </div>
      {phase !== null && <span className="text-xs text-muted-foreground">{t(`check.phase.${phase}`)}</span>}
    </div>
  );
}
