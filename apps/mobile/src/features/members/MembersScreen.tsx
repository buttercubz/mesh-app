// Members screen (WP-22, DESIGN §11.4, §1.3 F9): every rider with honest reachability,
// and for the leader: Remove and Lock group (each previewing the unconfirmed members who
// would lose access), Close group and Make leader.
import type { NodeRef } from "@meshapp/engine-api";
import { Button } from "@meshapp/ui-kit";
import { Lock, Navigation, XCircle } from "lucide-react";
import { useState } from "react";

import type { ScreenProps } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useIntercomEngine, useSession } from "../group/engine";
import { runCommand, showError } from "../group/session";
import { SessionGone } from "../group/SessionGone";
import { sortMembers } from "./format";
import type { LeaderAction } from "./LeaderSheets";
import { LeaderSheet } from "./LeaderSheets";
import { OpenMemberActions } from "../open/OpenMemberActions";
import { MemberRow } from "./MemberRow";

export function MembersScreen(_props: ScreenProps) {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const members = useEngineStore((s) => s.snapshot?.members ?? null);
  const [action, setAction] = useState<LeaderAction | null>(null);
  const [busy, setBusy] = useState(false);
  const leader = session?.role === "leader";
  const list = sortMembers(members ?? []);

  /** Fetches the unconfirmed members who would lose access, then shows `next`. */
  const preview = (next: (unconfirmed: readonly NodeRef[]) => LeaderAction) => {
    setBusy(true);
    engine
      .command({ type: "previewAccessLoss" })
      .then(
        (r) => {
          if (r.ok) setAction(next(r.unconfirmed));
          else showError(r.code);
        },
        (e: unknown) => {
          console.warn("[members] previewAccessLoss failed", e);
          showError(undefined);
        },
      )
      .finally(() => setBusy(false));
  };

  const confirm = (a: LeaderAction) => {
    const command =
      a.kind === "remove"
        ? ({ type: "removeMember", nodeId: a.member.nodeId } as const)
        : a.kind === "lock"
          ? ({ type: "lockGroup" } as const)
          : a.kind === "close"
            ? ({ type: "closeGroup" } as const)
            : a.kind === "transfer"
              ? ({ type: "transferLeader", nodeId: a.member.nodeId } as const)
              : null;
    if (command === null) return;
    setBusy(true);
    void runCommand(engine, command).then((ok) => {
      setBusy(false);
      if (ok) setAction(null);
    });
  };

  return (
    <ScreenLayout title={t("members.title", { n: list.length })}>
      {session === null ? (
        <SessionGone />
      ) : (
        <div className="mx-auto flex max-w-xl flex-col gap-4" data-testid="members-screen">
          <ul
            className="overflow-hidden rounded-section border border-separator bg-card text-card-foreground shadow-surface"
            aria-label={t("members.listLabel")}
          >
            {list.map((m) => (
              <MemberRow
                key={m.nodeId}
                member={m}
                {...(leader && m.status !== "me" ? { onOpen: () => setAction({ kind: "member", member: m }) } : {})}
                {...(session.openChannel !== null && m.status !== "me" ? { actions: <OpenMemberActions member={m} /> } : {})}
              />
            ))}
          </ul>
          {!session.shareDistance && (
            <p className="flex items-start gap-2 px-1 text-xs text-muted-foreground">
              <Navigation aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
              {t("members.distanceHint")}
            </p>
          )}
          {leader && (
            <section className="flex flex-col gap-2">
              <h2 className="px-4 text-xs font-semibold tracking-wider text-muted-foreground uppercase">{t("members.leaderTools")}</h2>
              <div className="flex flex-col gap-3 rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface">
                <Button
                  variant="secondary"
                  size="lg"
                  block
                  disabled={busy}
                  onClick={() => preview((unconfirmed) => ({ kind: "lock", unconfirmed }))}
                >
                  <Lock aria-hidden="true" />
                  {t("leader.lock")}
                </Button>
                <p className="-mt-1 px-1 text-xs text-muted-foreground">{t("leader.lockHint")}</p>
                <Button variant="danger" size="lg" block disabled={busy} onClick={() => setAction({ kind: "close" })}>
                  <XCircle aria-hidden="true" />
                  {t("leader.close")}
                </Button>
              </div>
            </section>
          )}
          {!leader && (
            <p className="px-1 text-xs text-muted-foreground">
              {t(session.openChannel !== null ? "open.membersHint" : "members.memberHint")}
            </p>
          )}
        </div>
      )}
      <LeaderSheet
        action={action}
        busy={busy}
        onClose={() => setAction(null)}
        onRemove={(member) => preview((unconfirmed) => ({ kind: "remove", member, unconfirmed }))}
        onTransfer={(member) => setAction({ kind: "transfer", member })}
        onConfirm={confirm}
      />
    </ScreenLayout>
  );
}
