// Lobby member list with ready checks (WP-22, DESIGN §1.3 F3): per member the link, mic
// (after Go live), headset route, battery and OS from PRESENCE, plus the unconfirmed
// badge (JOIN_REQ not yet seen by the leader) and the best-effort voice marking.
import type { Member } from "@meshapp/engine-api";
import { Avatar, Badge, cx } from "@meshapp/ui-kit";
import { Battery, Check as CheckIcon, Circle, Headphones } from "lucide-react";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { sortMembers, statusLabel, statusTone } from "../members/format";
import { RiderRowLayout } from "../members/RiderRowLayout";
import { batteryPercent, isConnected, routeKey } from "../ride/model";

function Check({ ok, label }: { readonly ok: boolean; readonly label: string }) {
  return (
    <span
      className={cx("inline-flex items-center gap-1", ok ? "text-foreground" : "text-muted-foreground")}
      data-ok={ok ? "true" : "false"}
    >
      {ok ? (
        <CheckIcon aria-hidden="true" className="size-3.5 text-talk-active" strokeWidth={3} />
      ) : (
        <Circle aria-hidden="true" className="size-3.5" />
      )}
      {label}
    </span>
  );
}

/** 44 px at the default text size, growing with it only up to 48 px: the name keeps the room. */
const ROW_AVATAR = "clamp(44px, 2.75rem, 48px)";

function ReadyRow({ m }: { readonly m: Member }) {
  const { t } = useT("group");
  const me = m.status === "me";
  const linked = me || isConnected(m);
  const ready = linked && m.ready.live && m.ready.mic;
  return (
    <li className="@container py-3" data-member={m.name} data-status={m.status}>
      <RiderRowLayout
        avatar={<Avatar name={m.name} size={ROW_AVATAR} dot={ready ? "active" : linked ? "brand" : "out"} dim={!linked} />}
        name={
          <span className="line-clamp-2 min-w-0 text-base leading-tight font-semibold break-words">
            {me ? t("member.you", { name: m.name }) : m.name}
          </span>
        }
        details={<ReadyDetails m={m} me={me} linked={linked} />}
      />
    </li>
  );
}

function ReadyDetails({ m, me, linked }: { readonly m: Member; readonly me: boolean; readonly linked: boolean }) {
  const { t } = useT("group");
  const { t: tr } = useT("ride");
  const status = statusLabel(m);
  const battery = batteryPercent(m.battery);
  return (
    <>
      <div className="flex flex-wrap items-center gap-1.5">
        {m.role === "leader" && <Badge tone="brand">{t("member.leader")}</Badge>}
        <Badge tone={statusTone(m.status)}>{t(status.key, status.params)}</Badge>
        {!me && linked && m.voice === "bestEffort" && <Badge tone="warn">{t("member.voice.bestEffort")}</Badge>}
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[0.8125rem]">
        <Check ok={linked} label={t("lobby.check.link")} />
        <Check ok={m.ready.live && m.ready.mic} label={t("lobby.check.mic")} />
        <span className="inline-flex items-center gap-1 text-muted-foreground">
          <Headphones aria-hidden="true" className="size-3.5" />
          {m.ready.route === null ? t("lobby.check.noRoute") : tr(routeKey(m.ready.route))}
        </span>
        {battery !== null && (
          <span className="inline-flex items-center gap-1 text-muted-foreground">
            <Battery aria-hidden="true" className="size-3.5" />
            {t("member.battery", { p: battery })}
          </span>
        )}
        <span className="text-muted-foreground">{t(`member.platform.${m.platform}`)}</span>
      </div>
    </>
  );
}

export function ReadyList() {
  const { t } = useT("group");
  const members = useEngineStore((s) => s.snapshot?.members ?? null);
  const list = sortMembers(members ?? []);
  return (
    <section
      className="rounded-section border border-separator bg-card px-4 py-2 text-card-foreground shadow-surface"
      aria-labelledby="ready-title"
    >
      <h2 id="ready-title" className="pt-2 text-base font-semibold">
        {t("lobby.riders", { n: list.length })}
      </h2>
      {list.length <= 1 && <p className="py-2 text-sm text-muted-foreground">{t("lobby.waitingRiders")}</p>}
      <ul data-testid="ready-list" className="divide-y divide-separator">
        {list.map((m) => (
          <ReadyRow key={m.nodeId} m={m} />
        ))}
      </ul>
    </section>
  );
}
