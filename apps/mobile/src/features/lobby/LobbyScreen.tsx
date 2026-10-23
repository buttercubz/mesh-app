// Lobby (WP-22, DESIGN §1.3 F3, §11.4): invite QR, member ready checks, "voice reaches
// N of M" (best effort marked), Go live (★, from the tap: capture starts muted and the
// screen says so) and Start ride (leader; a member may start locally when the leader is
// out of range). The screen is kept awake: phones must not lock before Go live.
import { Button } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import type { ScreenProps } from "../../app/router";
import { SCREEN, useRouter } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useBackHandler } from "../../components/useBackHandler";
import { useKeepScreenOn } from "../../components/useKeepScreenOn";
import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useIntercomEngine, useSession } from "../group/engine";
import { runCommand } from "../group/session";
import { SessionGone } from "../group/SessionGone";
import { runGesture } from "../ride/gestures";
import { LeaveSheet } from "../ride/LeaveSheet";
import { heardBy, isConnected, othersOf, voiceReach } from "../ride/model";
import { RideBanners } from "../ride/RideBanners";
import { InviteQr } from "./InviteQr";
import { ReadyList } from "./ReadyList";

function LobbyActions() {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const snapshot = useEngineStore((s) => s.snapshot);
  const [busy, setBusy] = useState(false);
  if (session === null) return null;
  const leader = session.role === "leader";
  const leaderAway = !leader && othersOf(snapshot).every((m) => m.role !== "leader" || !isConnected(m));

  const goLive = () => {
    // ★ Go live: issued synchronously inside this tap (gesture rule, §3.2).
    setBusy(true);
    void runGesture(engine, "goLive").finally(() => setBusy(false));
  };
  const startRide = () => {
    setBusy(true);
    void runCommand(engine, { type: "rideControl", op: "start" }).finally(() => setBusy(false));
  };

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col gap-2">
      {session.live ? (
        <p role="status" className="text-center text-sm font-medium" data-testid="mic-muted-status">
          <span aria-hidden="true" className="text-talk-active">
            ●{" "}
          </span>
          {t("lobby.liveMuted")}
        </p>
      ) : (
        <Button variant="primary" size="lg" block disabled={busy} onClick={goLive} data-testid="go-live">
          {t("lobby.goLive")}
        </Button>
      )}
      {leader ? (
        <Button
          variant={session.live ? "primary" : "secondary"}
          size="lg"
          block
          disabled={busy}
          onClick={startRide}
          data-testid="start-ride"
        >
          {t("lobby.startRide")}
        </Button>
      ) : (
        <>
          <p className="text-center text-sm text-muted-foreground">
            {leaderAway ? t("lobby.leaderAway") : t("lobby.waitLeader")}
          </p>
          <Button variant="secondary" size="lg" block disabled={busy} onClick={startRide} data-testid="start-ride">
            {t("lobby.startLocal")}
          </Button>
        </>
      )}
    </div>
  );
}

export function LobbyScreen({ params }: ScreenProps) {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const snapshot = useEngineStore((s) => s.snapshot);
  const [leaving, setLeaving] = useState(false);
  const groupId = session?.groupId ?? (typeof params["groupId"] === "string" ? params["groupId"] : null);

  useKeepScreenOn(true, engine);
  useBackHandler(() => {
    setLeaving(true);
    return true;
  });

  const ride = session?.ride ?? null;
  useEffect(() => {
    if (ride === "riding") useRouter.getState().replace(SCREEN.ride, groupId === null ? {} : { groupId });
  }, [ride, groupId]);

  const reach = voiceReach(snapshot);
  const heard = heardBy(snapshot);

  return (
    <ScreenLayout
      title={session?.groupName ?? t("lobby.title")}
      back
      onBack={() => setLeaving(true)}
      action={
        <Button variant="plain" onClick={() => useRouter.getState().push(SCREEN.members)}>
          {t("lobby.members")}
        </Button>
      }
    >
      {session === null ? (
        <SessionGone />
      ) : (
        <div className="mx-auto flex max-w-xl flex-col gap-4" data-testid="lobby-screen">
          <div className="text-center">
            <p className="text-xl font-semibold tabular-nums" data-testid="lobby-reach">
              {reach.bestEffort > 0
                ? t("lobby.voiceReachBestEffort", { n: reach.n, m: reach.m, b: reach.bestEffort })
                : t("lobby.voiceReach", { n: reach.n, m: reach.m })}
            </p>
            <p className="text-sm text-muted-foreground">{t("lobby.heardBy", { k: heard.k, n: heard.n })}</p>
          </div>
          <RideBanners hide={["notLive", "busy", "preempted", "congestion"]} />
          {/* Primary actions near the top: thumb-reachable without scrolling past the list. */}
          <LobbyActions />
          <InviteQr groupId={session.groupId} />
          <ReadyList />
          <p className="text-center text-xs text-muted-foreground">{t("lobby.keepOpen")}</p>
        </div>
      )}
      <LeaveSheet open={leaving} onClose={() => setLeaving(false)} />
    </ScreenLayout>
  );
}
