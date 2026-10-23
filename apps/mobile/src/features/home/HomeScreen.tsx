// Home (WP-22, DESIGN §11.4, §1.3 F1, F2, F7): the ride in progress (or Resume ride
// after process death), Create, Join, saved groups, Demo ride and Settings. On first
// launch it welcomes the rider and leads to onboarding.
import { Button, ListRow, ListSection } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import type { ScreenProps } from "../../app/router";
import { SCREEN, useRouter } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useT } from "../../i18n/index";
import { useGroupsStore } from "../../store/useGroupsStore";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine, useSession } from "../group/engine";
import { CREATE_GROUP_SCREEN } from "../group/ids";
import { openSession, refreshGroups, showError } from "../group/session";
import { Welcome } from "./Welcome";

function ActiveCard() {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const activeId = useGroupsStore((s) => s.activeGroupId);
  const active = useGroupsStore((s) => s.groups.find((g) => g.id === s.activeGroupId) ?? null);
  if (session !== null) {
    const riding = session.ride === "riding";
    return (
      <section className="rounded-section bg-card p-4 text-card-foreground" data-testid="home-active">
        <p className="text-sm text-muted-foreground">{riding ? t("home.riding") : t("home.inLobby")}</p>
        <h2 className="mb-3 text-2xl font-semibold">{session.groupName}</h2>
        <Button
          variant="primary"
          size="lg"
          block
          onClick={() => useRouter.getState().push(riding ? SCREEN.ride : SCREEN.lobby, { groupId: session.groupId })}
        >
          {riding ? t("home.backToRide") : t("home.backToLobby")}
        </Button>
      </section>
    );
  }
  if (activeId === null || active === null) return null;
  return (
    <section className="rounded-section bg-card p-4 text-card-foreground" data-testid="home-resume">
      <p className="text-sm text-muted-foreground">{t("home.interrupted")}</p>
      <h2 className="mb-3 text-2xl font-semibold">{active.name}</h2>
      <Button
        variant="primary"
        size="lg"
        block
        // ★ Resume ride: startEngine and goLive both leave inside this tap (§1.3 F7).
        onClick={() => void openSession(engine, { groupId: active.id, goLive: true })}
      >
        {t("home.resumeRide")}
      </Button>
      <Button variant="plain" block onClick={() => useGroupsStore.getState().setActive(null)}>
        {t("home.dismissResume")}
      </Button>
    </section>
  );
}

export function HomeScreen(_props: ScreenProps) {
  const { t } = useT("group");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const onboarded = useSettingsStore((s) => s.onboardingDone);
  const groups = useGroupsStore((s) => s.groups);
  const session = useSession();
  const [scanning, setScanning] = useState(false);

  useEffect(() => {
    if (onboarded) void refreshGroups(engine);
  }, [engine, onboarded]);

  const join = () => {
    setScanning(true);
    engine
      .scanInviteQr()
      .then(
        (r) => {
          if (!r.cancelled && r.url !== undefined) useRouter.getState().push(SCREEN.join, { url: r.url });
        },
        (e: unknown) => {
          console.warn("[home] scanInviteQr failed", e);
          showError(undefined);
        },
      )
      .finally(() => setScanning(false));
  };

  return (
    <ScreenLayout
      title={tc("appName")}
      back={false}
      action={
        <Button variant="plain" onClick={() => useRouter.getState().push(SCREEN.settings)}>
          {t("home.settings")}
        </Button>
      }
    >
      {!onboarded ? (
        <Welcome />
      ) : (
        <div className="mx-auto flex max-w-xl flex-col gap-5 pb-6" data-testid="home-screen">
          <ActiveCard />
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <Button variant="primary" size="lg" block onClick={() => useRouter.getState().push(CREATE_GROUP_SCREEN)}>
              {t("home.create")}
            </Button>
            <Button variant="secondary" size="lg" block disabled={scanning} onClick={join}>
              {t("home.join")}
            </Button>
          </div>
          {groups.length > 0 && (
            <ListSection title={t("home.savedGroups")}>
              {groups.map((g) => (
                <ListRow
                  key={g.id}
                  title={g.name}
                  subtitle={g.role === "leader" ? t("home.roleLeader") : t("home.roleMember")}
                  trailing={<span aria-hidden="true">›</span>}
                  disabled={session !== null}
                  ariaLabel={t("home.openGroup", { name: g.name })}
                  onClick={() => void openSession(engine, { groupId: g.id })}
                />
              ))}
            </ListSection>
          )}
          <section className="rounded-section bg-card p-4 text-card-foreground">
            <h2 className="text-base font-semibold">{t("home.demoTitle")}</h2>
            <p className="mt-1 mb-3 text-sm text-muted-foreground">{t("home.demoText")}</p>
            <Button
              variant="secondary"
              block
              disabled={session !== null}
              // Demo ride: startEngine({demo}) and goLive leave inside this tap.
              onClick={() => void openSession(engine, { demo: true, goLive: true })}
            >
              {t("home.demoStart")}
            </Button>
          </section>
          <p className="text-xs leading-relaxed text-muted-foreground">{t("home.promise")}</p>
        </div>
      )}
    </ScreenLayout>
  );
}
