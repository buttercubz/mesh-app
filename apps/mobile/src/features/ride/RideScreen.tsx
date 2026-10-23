// Ride screen (WP-22, DESIGN §11.4, §1.3 F4-F6): group name and "heard by k/n" on top,
// state banners, speaking chips in the centre, the PTT/Mute button, mode control and
// route chip at the bottom. No text input. The screen stays awake in PTT mode and, for
// the car profile, while "keep screen on while mounted" is on. Android back never
// leaves the ride by accident: it opens the Leave sheet.
import { Button } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

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
import { claimDefaultMode } from "./defaults";
import { LeaveSheet } from "./LeaveSheet";
import { heardBy, keepRideScreenOn, startMode, voiceReach, voxAllowed } from "./model";
import { RideBanners } from "./RideBanners";
import { TalkerChips } from "./TalkerChips";
import { TalkControl } from "./TalkControl";

export function RideScreen(_props: ScreenProps) {
  const { t } = useT("ride");
  const engine = useIntercomEngine();
  const session = useSession();
  const snapshot = useEngineStore((s) => s.snapshot);
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

  // Once per ride: the rider's default talk mode (§1.4), never VOX on the loudspeaker.
  useEffect(() => {
    if (groupId === null || ride !== "riding" || !live || mode === null) return;
    if (!claimDefaultMode(groupId)) return;
    const wanted = startMode(defaultMode, route);
    if (mode !== wanted) void runCommand(engine, { type: "setTalkMode", mode: wanted }, true);
  }, [groupId, ride, live, mode, defaultMode, route, engine]);

  // VOX is never kept on the phone loudspeaker (route changed mid-ride): fall back to PTT.
  useEffect(() => {
    if (mode === "vox" && !voxAllowed(route)) void runCommand(engine, { type: "setTalkMode", mode: "ptt" }, true);
  }, [mode, route, engine]);

  // The leader ended the ride: back to the lobby of the same group.
  useEffect(() => {
    if (ride === "lobby" && groupId !== null) useRouter.getState().replace(SCREEN.lobby, { groupId });
  }, [ride, groupId]);

  const heard = heardBy(snapshot);
  const reach = voiceReach(snapshot);

  return (
    <ScreenLayout
      title={session?.groupName ?? t("title")}
      onBack={() => setLeaving(true)}
      back
      action={
        <Button variant="plain" onClick={() => useRouter.getState().push(SCREEN.members)}>
          {t("members")}
        </Button>
      }
      footer={session !== null ? <TalkControl /> : undefined}
    >
      {session === null ? (
        <SessionGone />
      ) : (
        <div className="mx-auto flex max-w-xl flex-col gap-4" data-testid="ride-screen">
          <div className="flex flex-wrap items-baseline justify-center gap-x-4 gap-y-1 text-center">
            <p className="text-2xl font-semibold tabular-nums" data-testid="heard-by">
              {t("heardBy", { k: heard.k, n: heard.n })}
            </p>
            <p className="text-sm text-muted-foreground" data-testid="voice-reach">
              {reach.bestEffort > 0
                ? t("voiceReachBestEffort", { n: reach.n, m: reach.m, b: reach.bestEffort })
                : t("voiceReach", { n: reach.n, m: reach.m })}
            </p>
          </div>
          <RideBanners />
          <TalkerChips />
        </div>
      )}
      <LeaveSheet open={leaving} onClose={() => setLeaving(false)} />
    </ScreenLayout>
  );
}
