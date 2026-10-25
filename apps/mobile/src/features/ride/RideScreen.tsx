// Ride screen (WP-22, DESIGN §11.4, §1.3 F4-F6), built for a glance at speed: one status
// line on top ("Heard by k of n") whose chip expands into the details (voice reach,
// audio route, secondary alerts), at most one alert in full, who is talking big in the
// centre, the PTT/Mute button and the glove-sized mode control at the bottom. No text
// input. The screen stays awake in PTT mode and, for the car profile, while "keep screen
// on while mounted" is on. Android back never leaves the ride by accident: it opens the
// Leave sheet.
import { Badge, Button, cx } from "@meshapp/ui-kit";
import { ChevronDown, Globe, Headphones, Users } from "lucide-react";
import { useEffect, useId, useState } from "react";

import type { ScreenProps } from "../../app/router";
import { SCREEN, useRouter } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useBackHandler } from "../../components/useBackHandler";
import { useKeepScreenOn } from "../../components/useKeepScreenOn";
import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine, useSession } from "../group/engine";
import { runCommand } from "../group/session";
import { SessionGone } from "../group/SessionGone";
import { applyBlockedNames } from "../open/actions";
import { claimDefaultMode } from "./defaults";
import { LeaveSheet } from "./LeaveSheet";
import { heardBy, isLoudspeaker, keepRideScreenOn, routeKey, startMode, voiceReach, voxAllowed } from "./model";
import { AlertBanner } from "./RideBanners";
import { useRideBanners } from "./useRideBanners";
import { TalkerChips } from "./TalkerChips";
import { TalkControl } from "./TalkControl";

/**
 * The one status line: "Heard by k of n" big, and a compact chip (audio route, count of
 * collapsed alerts) that expands into the details. The most urgent alert shows in full
 * below it; never more than one.
 */
function RideStatus() {
  const { t } = useT("ride");
  const snapshot = useEngineStore((s) => s.snapshot);
  const route = snapshot?.audio.route ?? null;
  const { primary, rest } = useRideBanners();
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const heard = heardBy(snapshot);
  const reach = voiceReach(snapshot);
  const allHear = heard.n > 0 && heard.k === heard.n;
  const noRoute = route === null || route === "none";
  return (
    <div className="flex flex-col items-center gap-3">
      <p className="inline-flex items-center gap-2.5 text-3xl font-bold tracking-tight tabular-nums">
        <span
          aria-hidden="true"
          className={cx("size-3 shrink-0 rounded-full", allHear ? "bg-talk-active" : heard.k > 0 ? "bg-ladder-warn" : "bg-status-out")}
        />
        <span data-testid="heard-by">{t("heardBy", { k: heard.k, n: heard.n })}</span>
      </p>
      <button
        type="button"
        className="inline-flex min-h-target max-w-full items-center gap-2 rounded-capsule bg-control-fill px-4 text-sm font-semibold transition-transform duration-ui active:scale-[0.98]"
        aria-expanded={open}
        aria-controls={panelId}
        aria-label={`${t("status.details")}: ${t(routeKey(route))}`}
        onClick={() => setOpen((o) => !o)}
        data-testid="ride-status-chip"
      >
        <Headphones aria-hidden="true" className={cx("size-4 shrink-0", noRoute ? "text-danger" : "text-brand")} />
        <span className="truncate" data-testid="route-chip">
          {t(routeKey(route))}
        </span>
        {rest.length > 0 && <Badge tone="warn">{t("alerts.more", { n: rest.length })}</Badge>}
        {rest.length === 0 && isLoudspeaker(route) && <Badge tone="neutral">{t("mode.ptt")}</Badge>}
        <ChevronDown aria-hidden="true" className={cx("size-4 shrink-0 transition-transform duration-ui", open && "rotate-180")} />
      </button>
      {open && (
        <div id={panelId} className="flex w-full animate-rise flex-col gap-2 motion-reduce:animate-none" data-testid="ride-status-details">
          <p className="rounded-xl bg-card px-4 py-3 text-center text-sm text-card-foreground" data-testid="voice-reach">
            {reach.bestEffort > 0
              ? t("voiceReachBestEffort", { n: reach.n, m: reach.m, b: reach.bestEffort })
              : t("voiceReach", { n: reach.n, m: reach.m })}
          </p>
          {isLoudspeaker(route) && (
            <p className="px-2 text-center text-xs text-muted-foreground" data-testid="vox-speaker-note">
              {t("mode.voxSpeaker")}
            </p>
          )}
          {rest.map((b) => (
            <AlertBanner key={b.kind} banner={b} compact />
          ))}
        </div>
      )}
      {primary !== null && (
        <div className="w-full" data-testid="ride-banners">
          <AlertBanner banner={primary} />
        </div>
      )}
    </div>
  );
}

/** Open channels: the public notice stays on the ride screen. */
function PublicNotice() {
  const { t } = useT("group");
  return (
    <p
      className="flex items-start gap-2 rounded-xl border border-ladder-warn/40 bg-ladder-warn/10 px-4 py-3 text-sm font-semibold"
      role="note"
      data-testid="open-public-notice"
    >
      <Globe aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-ladder-warn" />
      {t("open.public")}
    </p>
  );
}

export function RideScreen(_props: ScreenProps) {
  const { t } = useT("ride");
  const { t: tg } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const route = useEngineStore((s) => s.snapshot?.audio.route ?? null);
  const vehicle = useSettingsStore((s) => s.vehicle);
  const keepOnMounted = useSettingsStore((s) => s.keepScreenOnMounted);
  const defaultMode = useSettingsStore((s) => s.defaultMode);
  const [leaving, setLeaving] = useState(false);

  useKeepScreenOn(keepRideScreenOn(session?.mode, vehicle, keepOnMounted), engine);
  useBackHandler(() => {
    setLeaving(true);
    return true;
  });

  const groupId = session?.groupId ?? null;
  const ride = session?.ride ?? null;
  const live = session?.live ?? false;
  const mode = session?.mode ?? null;
  const openChannel = session?.openChannel ?? null;
  const members = useEngineStore((s) => s.snapshot?.members ?? null);

  // Open channels: re-apply this device's blocked names to riders heard now.
  useEffect(() => {
    if (openChannel !== null && members !== null) applyBlockedNames(engine);
  }, [openChannel, members, engine]);

  // Once per ride: the rider's default talk mode (§1.4), never VOX on the loudspeaker.
  useEffect(() => {
    // Open channels start in PTT (public: never an open mic by default).
    if (groupId === null || ride !== "riding" || !live || mode === null || openChannel !== null) return;
    if (!claimDefaultMode(groupId)) return;
    const wanted = startMode(defaultMode, route);
    if (mode !== wanted) void runCommand(engine, { type: "setTalkMode", mode: wanted }, true);
  }, [groupId, ride, live, mode, defaultMode, route, engine, openChannel]);

  // VOX is never kept on the phone loudspeaker (route changed mid-ride): fall back to PTT.
  useEffect(() => {
    if (mode === "vox" && !voxAllowed(route)) void runCommand(engine, { type: "setTalkMode", mode: "ptt" }, true);
  }, [mode, route, engine]);

  // The leader ended the ride: back to the lobby of the same group.
  useEffect(() => {
    if (ride === "lobby" && groupId !== null) useRouter.getState().replace(SCREEN.lobby, { groupId });
  }, [ride, groupId]);

  return (
    <ScreenLayout
      title={openChannel !== null ? tg("open.channel", { n: openChannel }) : (session?.groupName ?? t("title"))}
      onBack={() => setLeaving(true)}
      back
      action={
        <Button
          variant="plain"
          iconOnly
          aria-label={t("members")}
          className="text-foreground"
          onClick={() => useRouter.getState().push(SCREEN.members)}
        >
          <Users aria-hidden="true" />
        </Button>
      }
      footer={session !== null ? <TalkControl /> : undefined}
    >
      {session === null ? (
        <SessionGone />
      ) : (
        <div className="mx-auto flex min-h-full max-w-xl flex-col gap-6 pb-2" data-testid="ride-screen">
          <RideStatus />
          {openChannel !== null && <PublicNotice />}
          <div className="flex flex-1 items-start justify-center pt-2">
            <TalkerChips />
          </div>
        </div>
      )}
      <LeaveSheet open={leaving} onClose={() => setLeaving(false)} />
    </ScreenLayout>
  );
}
