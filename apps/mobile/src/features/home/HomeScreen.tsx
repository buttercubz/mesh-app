// Home (WP-22, DESIGN §11.4, §1.3 F1, F2, F7): one clear primary action (Start a ride,
// i.e. create a group) with Join a ride beside it, the ride in progress (or Resume ride
// after process death) on top when there is one, saved groups as cards, Demo ride and
// Settings. On first launch it welcomes the rider and leads to onboarding.
import { Avatar, Button, ListRow, ListSection } from "@meshapp/ui-kit";
import { ChevronRight, Plus, Radio, ScanLine, Settings, Sparkles } from "lucide-react";
import type { ReactNode } from "react";
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
import { OpenChannelCard } from "../open/OpenChannelCard";
import { BrandMark, Welcome } from "./Welcome";

function ActiveCard() {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const session = useSession();
  const activeId = useGroupsStore((s) => s.activeGroupId);
  const active = useGroupsStore((s) => s.groups.find((g) => g.id === s.activeGroupId) ?? null);
  if (session !== null) {
    const riding = session.ride === "riding";
    return (
      <section
        className="animate-rise rounded-section border border-talk-active/40 bg-talk-active/10 p-4 text-card-foreground motion-reduce:animate-none"
        data-testid="home-active"
      >
        <p className="flex items-center gap-2 text-sm font-semibold text-talk-active">
          <span aria-hidden="true" className="size-2 animate-talk-pulse rounded-full bg-talk-active motion-reduce:animate-none" />
          {riding ? t("home.riding") : t("home.inLobby")}
        </p>
        <h2 className="mt-1 mb-3 text-2xl font-bold tracking-tight">{session.groupName}</h2>
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
    <section
      className="animate-rise rounded-section border border-ladder-warn/40 bg-ladder-warn/10 p-4 text-card-foreground motion-reduce:animate-none"
      data-testid="home-resume"
    >
      <p className="text-sm font-semibold text-muted-foreground">{t("home.interrupted")}</p>
      <h2 className="mt-1 mb-3 text-2xl font-bold tracking-tight">{active.name}</h2>
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

/** A big tappable tile: icon disc, label, one-line hint. The accessible name is the label. */
function ActionTile({
  icon,
  label,
  hint,
  primary = false,
  disabled = false,
  onClick,
}: {
  readonly icon: ReactNode;
  readonly label: string;
  readonly hint: string;
  readonly primary?: boolean;
  readonly disabled?: boolean;
  readonly onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className={
        primary
          ? "flex min-h-28 w-full items-center gap-4 rounded-section bg-linear-to-br from-brand to-brand-strong p-5 text-start text-brand-foreground shadow-[0_18px_40px_-18px_var(--brand-glow)] transition-transform duration-ui active:scale-[0.98] disabled:opacity-50"
          : "flex min-h-20 w-full items-center gap-4 rounded-section border border-separator bg-card p-4 text-start text-card-foreground shadow-surface transition-transform duration-ui active:scale-[0.98] disabled:opacity-50"
      }
    >
      <span
        aria-hidden="true"
        className={
          primary
            ? "grid size-14 shrink-0 place-items-center rounded-full bg-white/20"
            : "grid size-12 shrink-0 place-items-center rounded-full bg-brand-soft text-brand"
        }
      >
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className={primary ? "text-xl font-bold tracking-tight" : "text-lg font-semibold"}>{label}</span>
        <span className={primary ? "text-sm opacity-85" : "text-sm text-muted-foreground"}>{hint}</span>
      </span>
      <ChevronRight aria-hidden="true" className="size-5 shrink-0 opacity-70" />
    </button>
  );
}

/** No saved groups yet: a small road illustration instead of an empty list. */
function EmptyGroups() {
  const { t } = useT("group");
  return (
    <div className="flex flex-col items-center gap-3 rounded-section border border-dashed border-separator px-6 py-8 text-center" data-testid="home-empty">
      <svg viewBox="0 0 160 72" className="h-16 w-40 text-brand" aria-hidden="true" fill="none">
        <path d="M8 60 C 48 60, 56 16, 96 16 S 140 40, 152 12" stroke="currentColor" strokeOpacity="0.25" strokeWidth="10" strokeLinecap="round" />
        <path d="M8 60 C 48 60, 56 16, 96 16 S 140 40, 152 12" stroke="currentColor" strokeWidth="2" strokeDasharray="4 7" strokeLinecap="round" />
        <circle cx="34" cy="55" r="7" fill="currentColor" />
        <circle cx="96" cy="16" r="7" fill="currentColor" fillOpacity="0.7" />
        <circle cx="140" cy="24" r="7" fill="currentColor" fillOpacity="0.45" />
      </svg>
      <p className="text-base font-semibold">{t("home.emptyTitle")}</p>
      <p className="max-w-xs text-sm text-muted-foreground">{t("home.emptyText")}</p>
    </div>
  );
}

export function HomeScreen(_props: ScreenProps) {
  const { t } = useT("group");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const onboarded = useSettingsStore((s) => s.onboardingDone);
  const displayName = useSettingsStore((s) => s.displayName);
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
      title={<BrandMark label={tc("appName")} />}
      back={false}
      largeTitle
      glow
      action={
        <Button
          variant="plain"
          iconOnly
          aria-label={t("home.settings")}
          className="text-foreground"
          onClick={() => useRouter.getState().push(SCREEN.settings)}
        >
          <Settings aria-hidden="true" />
        </Button>
      }
    >
      {!onboarded ? (
        <Welcome />
      ) : (
        <div className="mx-auto flex max-w-xl flex-col gap-6 pb-6" data-testid="home-screen">
          <div className="flex flex-col gap-1 px-1 pt-2">
            <p className="text-3xl font-bold tracking-tight">
              {displayName.trim() === "" ? tc("appName") : t("home.greeting", { name: displayName })}
            </p>
            <p className="text-base text-muted-foreground">{t("home.subtitle")}</p>
          </div>
          <ActiveCard />
          <div className="flex flex-col gap-3">
            <ActionTile
              primary
              icon={<Plus className="size-7" strokeWidth={2.5} />}
              label={t("home.create")}
              hint={t("home.createHint")}
              onClick={() => useRouter.getState().push(CREATE_GROUP_SCREEN)}
            />
            <ActionTile
              icon={<ScanLine className="size-6" />}
              label={t("home.join")}
              hint={t("home.joinHint")}
              disabled={scanning}
              onClick={join}
            />
          </div>
          <OpenChannelCard disabled={session !== null} />
          {groups.length > 0 ? (
            <ListSection title={t("home.savedGroups")}>
              {groups.map((g) => (
                <ListRow
                  key={g.id}
                  leading={<Avatar name={g.name} size={44} />}
                  title={g.name}
                  subtitle={g.role === "leader" ? t("home.roleLeader") : t("home.roleMember")}
                  trailing={<ChevronRight aria-hidden="true" />}
                  disabled={session !== null}
                  ariaLabel={t("home.openGroup", { name: g.name })}
                  onClick={() => void openSession(engine, { groupId: g.id })}
                />
              ))}
            </ListSection>
          ) : (
            <EmptyGroups />
          )}
          <section className="flex flex-col gap-4 rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface">
            <div className="flex items-center gap-4">
              <span aria-hidden="true" className="grid size-12 shrink-0 place-items-center rounded-full bg-brand-soft text-brand">
                <Sparkles className="size-6" />
              </span>
              <div className="min-w-0 flex-1">
                <h2 className="text-base font-semibold">{t("home.demoTitle")}</h2>
                <p className="text-sm text-muted-foreground">{t("home.demoText")}</p>
              </div>
            </div>
            <Button
              variant="secondary"
              block
              disabled={session !== null}
              // Demo ride: startEngine({demo}) and goLive leave inside this tap.
              onClick={() => void openSession(engine, { demo: true, goLive: true })}
            >
              <Radio aria-hidden="true" />
              {t("home.demoStart")}
            </Button>
          </section>
          <p className="px-1 text-xs leading-relaxed text-muted-foreground">{t("home.promise")}</p>
        </div>
      )}
    </ScreenLayout>
  );
}
