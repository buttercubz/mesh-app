// One member of the Members screen (WP-22, DESIGN §11.4): status and hops, voice reach
// (clear / best effort / no), path quality, hears-me / one-way, flags (on a call, mic
// paused, no headset, radio off), battery, platform and, when both share, distance and
// bearing. Colour is always paired with text.
import type { Member } from "@meshapp/engine-api";
import { Badge, QualityBars } from "@meshapp/ui-kit";

import { INTL_LOCALE, useT } from "../../i18n/index";
import { batteryPercent, compassKey, formatDistance, isConnected, qualityBars } from "../ride/model";
import { flagTone, sortedFlags, statusLabel, statusTone, voiceKey, voiceTone } from "./format";

export interface MemberRowProps {
  readonly member: Member;
  /** Leader actions for this member (opens the member sheet). */
  readonly onOpen?: () => void;
}

export function MemberRow({ member: m, onOpen }: MemberRowProps) {
  const { t, language } = useT("group");
  const me = m.status === "me";
  const status = statusLabel(m);
  const battery = batteryPercent(m.battery);
  const flags = sortedFlags(m.flags);
  const bars = isConnected(m) ? qualityBars(m.quality) : 0;
  const reach = me || !isConnected(m) ? null : m.hearsMe ? t("member.hearsMe") : t("member.notHearsMe");
  const voice = me ? "kpi" : m.status === "lost" || m.status === "unconfirmed" ? "no" : m.voice;

  const body = (
    <>
      <span className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 truncate text-lg font-semibold">{me ? t("member.you", { name: m.name }) : m.name}</span>
        {m.role === "leader" && <Badge tone="brand">{t("member.leader")}</Badge>}
        <Badge tone={statusTone(m.status)}>{t(status.key, status.params)}</Badge>
        {!me && <Badge tone={voiceTone(voice)}>{t(voiceKey(m))}</Badge>}
      </span>
      {flags.length > 0 && (
        <span className="flex flex-wrap gap-2">
          {flags.map((f) => (
            <Badge key={f} tone={flagTone(f)} data-flag={f}>
              {t(`member.flag.${f}`)}
            </Badge>
          ))}
        </span>
      )}
      <span className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-muted-foreground">
        {!me && <QualityBars bars={bars} label={t("member.quality", { n: bars })} />}
        {reach !== null && <span>{reach}</span>}
        {m.distanceM !== null && (
          <span data-testid="member-distance">
            {m.bearingDeg !== null
              ? t("member.distanceBearing", {
                  d: formatDistance(m.distanceM, INTL_LOCALE[language]),
                  dir: t(compassKey(m.bearingDeg)),
                })
              : formatDistance(m.distanceM, INTL_LOCALE[language])}
          </span>
        )}
        {battery !== null && <span>{t("member.battery", { p: battery })}</span>}
        <span>{t(`member.platform.${m.platform}`)}</span>
      </span>
    </>
  );

  return (
    <li className="border-b border-separator last:border-b-0" data-member={m.name} data-status={m.status}>
      {onOpen !== undefined ? (
        <button
          type="button"
          className="flex min-h-target w-full flex-col items-start gap-1.5 px-4 py-3 text-start active:bg-control-fill"
          onClick={onOpen}
          aria-haspopup="dialog"
        >
          {body}
        </button>
      ) : (
        <div className="flex flex-col gap-1.5 px-4 py-3">{body}</div>
      )}
    </li>
  );
}
