// Ride screen (WP-22, DESIGN §11.4, §1.3 F4-F6), built for a glance at speed: one status
// row on top ("Heard by k of n" and a chip that opens the details sheet: voice reach,
// audio route, every alert in full), at most one alert as a compact bar, who is talking
// big in the centre, the PTT/Mute button and the glove-sized mode control at the bottom.
// The body is a fixed layout that never scrolls: what a rider glances at is never pushed
// under the footer; the talkers area shrinks instead (TalkerChips). No text input. The screen stays awake in PTT mode and, for the car profile, while "keep screen
// on while mounted" is on. Android back never leaves the ride by accident: it opens the
// Leave sheet.
import { Badge, Button, Sheet, cx } from "@meshapp/ui-kit";
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
 * The one status row: "Heard by k of n" big and a compact chip (audio route, or the count
 * of collapsed alerts) that opens the details sheet. The most urgent alert shows below it
 * as a bar (title and fix; its text opens the same sheet); never more than one.
 */
function RideStatus() {
  const { t } = useT("ride");
  const { t: tc } = useT("common");
  const { t: tg } = useT("group");
  const openChannel = useSession()?.openChannel ?? null;
  const snapshot = useEngineStore((s) => s.snapshot);
  const route = snapshot?.audio.route ?? null;
  const { primary, rest } = useRideBanners();
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const heard = heardBy(snapshot);
  const reach = voiceReach(snapshot);
  const allHear = heard.n > 0 && heard.k === heard.n;
  const noRoute = route === null || route === "none";
  const routeName = t(routeKey(route));
  return (
    <div className="flex shrink-0 flex-col gap-2">
      <div className="flex flex-wrap items-center justify-center gap-x-3 gap-y-1">
        <p className="inline-flex items-center gap-2.5 text-2xl font-bold tracking-tight whitespace-nowrap tabular-nums short-window:text-xl [@media(min-height:740px)]:text-3xl">
          <span
            aria-hidden="true"
            className={cx("size-3 shrink-0 rounded-full", allHear ? "bg-talk-active" : heard.k > 0 ? "bg-ladder-warn" : "bg-status-out")}
          />
          <span data-testid="heard-by">{t("heardBy", { k: heard.k, n: heard.n })}</span>
        </p>
        <button
          type="button"
          className="inline-flex min-h-target max-w-full min-w-0 items-center gap-2 rounded-capsule bg-control-fill px-4 text-sm font-semibold transition-transform duration-ui active:scale-[0.98]"
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-controls={panelId}
          aria-label={`${t("status.details")}: ${routeName}`}
          onClick={() => setOpen((o) => !o)}
          data-testid="ride-status-chip"
        >
          <Headphones aria-hidden="true" className={cx("size-4 shrink-0", noRoute ? "text-danger" : "text-brand")} />
          {/* With collapsed alerts the chip shows their count; the route is in the sheet. */}
          {rest.length === 0 && (
            <span className="min-w-0 truncate short-window:sr-only" data-testid="route-chip" title={routeName}>
              {routeName}
            </span>
          )}
          {rest.length > 0 && (
            <Badge tone="warn" className="shrink-0">
              {t("alerts.more", { n: rest.length })}
            </Badge>
          )}
          {rest.length === 0 && isLoudspeaker(route) && (
            <Badge tone="neutral" className="shrink-0">
              {t("mode.ptt")}
            </Badge>
          )}
          <ChevronDown aria-hidden="true" className="size-4 shrink-0" />
        </button>
      </div>
      {primary !== null && (
        <div className="w-full" data-testid="ride-banners">
          <AlertBanner banner={primary} bar onExpand={() => setOpen(true)} expanded={open} controls={panelId} />
        </div>
      )}
      <Sheet open={open} onClose={() => setOpen(false)} title={t("status.details")} closeLabel={tc("action.close")}>
        <div id={panelId} className="flex flex-col gap-3" data-testid="ride-status-details">
          <p className="rounded-xl bg-card px-4 py-3 text-center text-sm text-card-foreground" data-testid="voice-reach">
            {reach.bestEffort > 0
              ? t("voiceReachBestEffort", { n: reach.n, m: reach.m, b: reach.bestEffort })
              : t("voiceReach", { n: reach.n, m: reach.m })}
          </p>
          <p className="flex items-center justify-center gap-2 text-sm font-semibold" data-testid="route-detail">
            <Headphones aria-hidden="true" className={cx("size-4 shrink-0", noRoute ? "text-danger" : "text-brand")} />
            {routeName}
          </p>
          {isLoudspeaker(route) && (
            <p className="px-2 text-center text-xs text-muted-foreground" data-testid="vox-speaker-note">
              {t("mode.voxSpeaker")}
            </p>
          )}
          {openChannel !== null && (
            <p className="rounded-xl border border-ladder-warn/40 bg-ladder-warn/10 px-4 py-3 text-sm" data-testid="open-public-detail">
              <span className="block font-semibold">{tg("open.public")}</span>
              {tg("open.publicDetail")}
            </p>
          )}
          {primary !== null && <AlertBanner banner={primary} />}
          {rest.map((b) => (
            <AlertBanner key={b.kind} banner={b} />
          ))}
        </div>
      </Sheet>
    </div>
  );
}

/**
 * Open channels: the public notice stays on the ride screen (two lines at most; the full
 * text, with what not to share, is in the details sheet).
 */
function PublicNotice() {
  const { t } = useT("group");
  return (
    <p
      className="flex shrink-0 items-start gap-2 rounded-xl border border-ladder-warn/40 bg-ladder-warn/10 px-3 py-2 text-[0.8125rem] leading-snug font-semibold"
      role="note"
      data-testid="open-public-notice"
    >
      <Globe aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-ladder-warn" />
      <span className="line-clamp-2 text-pretty" title={t("open.public")}>
        {t("open.public")}
      </span>
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
      scroll={session === null}
    >
      {session === null ? (
        <SessionGone />
      ) : (
        <div className="mx-auto flex min-h-0 w-full max-w-xl flex-1 flex-col gap-3 short-window:gap-2" data-testid="ride-screen">
          <RideStatus />
          {openChannel !== null && <PublicNotice />}
          {/* A size container: the talkers adapt to the height left (TalkerChips). */}
          <div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden [container-type:size]">
            <TalkerChips />
          </div>
        </div>
      )}
      <LeaveSheet open={leaving} onClose={() => setLeaving(false)} />
    </ScreenLayout>
  );
}
