// One member of the Members screen (WP-22, DESIGN §11.4): status and hops, voice reach
// (clear / best effort / no), path quality, hears-me / one-way, flags (on a call, mic
// paused, no headset, radio off), battery, platform and, when both share, distance and
// bearing. Colour is always paired with text.
import type { Member } from "@meshapp/engine-api";
import type { AvatarDotTone } from "@meshapp/ui-kit";
import { Avatar, Badge, QualityBars } from "@meshapp/ui-kit";
import { Battery, ChevronRight, Crown, Navigation } from "lucide-react";
import type { ReactNode } from "react";

import { INTL_LOCALE, useT } from "../../i18n/index";
import { batteryPercent, compassKey, formatDistance, isConnected, qualityBars } from "../ride/model";
import { flagTone, sortedFlags, statusLabel, statusTone, transportKey, voiceKey, voiceTone } from "./format";
import { RiderRowLayout } from "./RiderRowLayout";

/** 48 px at the default text size, growing with it only up to 52 px: the name keeps the room. */
const ROW_AVATAR = "clamp(48px, 3rem, 52px)";

export interface MemberRowProps {
  readonly member: Member;
  /** Leader actions for this member (opens the member sheet). */
  readonly onOpen?: () => void;
  /** Extra controls under the row (open channels: local mute / block). */
  readonly actions?: ReactNode;
}

export function MemberRow({ member: m, onOpen, actions }: MemberRowProps) {
  const { t, language } = useT("group");
  const me = m.status === "me";
  const status = statusLabel(m);
  const via = me ? null : transportKey(m);
  const battery = batteryPercent(m.battery);
  const flags = sortedFlags(m.flags);
  const bars = isConnected(m) ? qualityBars(m.quality) : 0;
  const reach = me || !isConnected(m) ? null : m.hearsMe ? t("member.hearsMe") : t("member.notHearsMe");
  const voice = me ? "kpi" : m.status === "lost" || m.status === "unconfirmed" ? "no" : m.voice;

  const tone = statusTone(m.status);
  const dot: AvatarDotTone = me ? "brand" : tone === "neutral" || tone === "blocked" ? "out" : tone;

  const name = (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="line-clamp-2 min-w-0 text-lg leading-tight font-semibold break-words">
        {me ? t("member.you", { name: m.name }) : m.name}
      </span>
      {m.role === "leader" && <Crown aria-hidden="true" className="size-4 shrink-0 text-brand" />}
    </span>
  );
  const details = (
    <>
      <span className="flex flex-wrap items-center gap-1.5">
        {m.role === "leader" && <Badge tone="brand">{t("member.leader")}</Badge>}
        <Badge tone={statusTone(m.status)}>{t(status.key, status.params)}</Badge>
        {!me && <Badge tone={voiceTone(voice)}>{t(voiceKey(m))}</Badge>}
        {via !== null && (
          <Badge tone="neutral" data-transport={via}>
            {t(via)}
          </Badge>
        )}
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
      <span className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[0.8125rem] text-muted-foreground">
        {!me && <QualityBars bars={bars} label={t("member.quality", { n: bars })} />}
        {reach !== null && <span>{reach}</span>}
        {m.distanceM !== null && (
          <span className="inline-flex items-center gap-1">
            <Navigation aria-hidden="true" className="size-3.5" />
            <span data-testid="member-distance">
            {m.bearingDeg !== null
              ? t("member.distanceBearing", {
                  d: formatDistance(m.distanceM, INTL_LOCALE[language]),
                  dir: t(compassKey(m.bearingDeg)),
                })
              : formatDistance(m.distanceM, INTL_LOCALE[language])}
            </span>
          </span>
        )}
        {battery !== null && (
          <span className="inline-flex items-center gap-1">
            <Battery aria-hidden="true" className="size-3.5" />
            {t("member.battery", { p: battery })}
          </span>
        )}
        <span>{t(`member.platform.${m.platform}`)}</span>
      </span>
    </>
  );
  const avatar = <Avatar name={m.name} size={ROW_AVATAR} dot={dot} dim={m.status === "lost"} />;

  return (
    <li className="border-b border-separator last:border-b-0" data-member={m.name} data-status={m.status}>
      {onOpen !== undefined ? (
        <button
          type="button"
          className="@container flex min-h-target w-full px-4 py-3.5 text-start transition-colors duration-ui active:bg-control-fill"
          onClick={onOpen}
          aria-haspopup="dialog"
        >
          <RiderRowLayout
            avatar={avatar}
            name={name}
            details={details}
            trailing={<ChevronRight aria-hidden="true" className="size-5 shrink-0 text-muted-foreground" />}
          />
        </button>
      ) : (
        <div className="@container px-4 py-3.5">
          <RiderRowLayout avatar={avatar} name={name} details={details} />
        </div>
      )}
      {actions}
    </li>
  );
}
