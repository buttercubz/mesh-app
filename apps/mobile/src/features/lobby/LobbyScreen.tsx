// Lobby (WP-22, DESIGN §1.3 F3, §11.4): the first half of one continuous flow with the
// ride screen. The riders gathering (avatars), "voice reaches N of M" (best effort
// marked), the invite QR and the ready checks scroll; Go live (★, from the tap: capture
// starts muted and the screen says so) and Start ride (leader; a member may start
// locally when the leader is out of range) stay pinned where the PTT button will be;
// the member's note on who starts sits in the body so the footer stays short.
// The screen is kept awake: phones must not lock before Go live.
import { Avatar, Button, cx } from "@meshapp/ui-kit";
import { Flag, Mic, Users } from "lucide-react";
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

/**
 * Glove-sized primary actions that still leave the body room on short phones: the
 * standard large button (3.5 rem) there, the taller hero size from 700 px of height up.
 */
const HERO_ACTION = "min-h-14 [@media(min-height:700px)]:min-h-16 [@media(min-height:700px)]:text-lg";

/** A member's note on who starts the ride: in the body, so the pinned footer stays short. */
function MemberStartHint() {
  const { t } = useT("group");
  const session = useSession();
  const snapshot = useEngineStore((s) => s.snapshot);
  if (session === null || session.role === "leader") return null;
  const leaderAway = othersOf(snapshot).every((m) => m.role !== "leader" || !isConnected(m));
  return (
    <p className="text-center text-sm text-pretty text-muted-foreground" data-testid="lobby-start-hint">
      {leaderAway ? t("lobby.leaderAway") : t("lobby.waitLeader")}
    </p>
  );
}

function LobbyActions() {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const [busy, setBusy] = useState(false);
  if (session === null) return null;
  const leader = session.role === "leader";

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
    <div className="mx-auto flex w-full max-w-md flex-col gap-3">
      {session.live ? (
        <p
          role="status"
          className="flex min-h-target items-center justify-center gap-2 rounded-capsule bg-talk-active/12 px-4 text-center text-sm font-semibold"
          data-testid="mic-muted-status"
        >
          <span aria-hidden="true" className="size-2.5 shrink-0 animate-talk-pulse rounded-full bg-talk-active motion-reduce:animate-none" />
          {t("lobby.liveMuted")}
        </p>
      ) : (
        <Button variant="primary" size="lg" block disabled={busy} onClick={goLive} data-testid="go-live" className={HERO_ACTION}>
          <Mic aria-hidden="true" />
          {t("lobby.goLive")}
        </Button>
      )}
      <Button
        variant={leader && session.live ? "primary" : "secondary"}
        size="lg"
        block
        disabled={busy}
        onClick={startRide}
        data-testid="start-ride"
        className={cx(leader && session.live && HERO_ACTION)}
      >
        <Flag aria-hidden="true" />
        {leader ? t("lobby.startRide") : t("lobby.startLocal")}
      </Button>
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
  const members = snapshot?.members ?? [];
  const faces = [...members].sort((a, b) => Number(b.status === "me") - Number(a.status === "me")).slice(0, 6);

  return (
    <ScreenLayout
      title={session?.groupName ?? t("lobby.title")}
      back
      glow
      onBack={() => setLeaving(true)}
      action={
        <Button
          variant="plain"
          iconOnly
          aria-label={t("lobby.members")}
          className="text-foreground"
          onClick={() => useRouter.getState().push(SCREEN.members)}
        >
          <Users aria-hidden="true" />
        </Button>
      }
      footer={session !== null ? <LobbyActions /> : undefined}
    >
      {session === null ? (
        <SessionGone />
      ) : (
        <div className="mx-auto flex max-w-xl flex-col gap-5" data-testid="lobby-screen">
          <section className="flex flex-col items-center gap-4 pt-2 text-center">
            <ul className="flex -space-x-3" aria-hidden="true">
              {faces.map((m) => (
                <li key={m.nodeId} className="rounded-full ring-4 ring-background">
                  <Avatar name={m.name} size={56} dim={m.status === "lost" || m.status === "unconfirmed"} />
                </li>
              ))}
            </ul>
            <div className="flex flex-col gap-1">
              <p className="text-2xl font-bold tracking-tight tabular-nums" data-testid="lobby-reach">
                {reach.bestEffort > 0
                  ? t("lobby.voiceReachBestEffort", { n: reach.n, m: reach.m, b: reach.bestEffort })
                  : t("lobby.voiceReach", { n: reach.n, m: reach.m })}
              </p>
              <p className="text-sm text-muted-foreground">{t("lobby.heardBy", { k: heard.k, n: heard.n })}</p>
            </div>
            <MemberStartHint />
          </section>
          <RideBanners hide={["notLive", "busy", "preempted", "congestion"]} />
          <InviteQr groupId={session.groupId} />
          <ReadyList />
          <p className="px-4 text-center text-xs text-muted-foreground">{t("lobby.keepOpen")}</p>
        </div>
      )}
      <LeaveSheet open={leaving} onClose={() => setLeaving(false)} />
    </ScreenLayout>
  );
}
