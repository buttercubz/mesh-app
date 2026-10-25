// Onboarding steps 1-2 (WP-22, DESIGN §1.3 F0): "About you" (language, name, vehicle)
// and one permissions step: the walkthrough (each with a one-line reason, all in the
// foreground) plus the "Share my distance" choice with its keep-alive explanation.
import type { Vehicle } from "@meshapp/engine-api";
import { Button, SegmentedControl, cx } from "@meshapp/ui-kit";
import type { LucideIcon } from "lucide-react";
import { Bell, Bike, Bluetooth, Car, Check, MapPin, Mic } from "lucide-react";
import { useEffect, useState } from "react";

import type { IntercomPermissionStatus } from "../../engine/client";
import type { AppLanguage } from "../../i18n/index";
import { APP_LANGUAGES, useT } from "../../i18n/index";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { NameField } from "../group/NameField";
import { ONBOARDING_PERMISSIONS } from "./permissions";

export interface StepProps {
  readonly onNext: () => void;
}

/** Language picker as two pressable chips (applied at once, no step of its own). */
function LanguagePicker() {
  const { t } = useT("onboarding");
  const language = useSettingsStore((s) => s.language);
  return (
    <div className="flex flex-col gap-2">
      <span className="text-sm font-semibold">{t("language.question")}</span>
      <div className="grid grid-cols-2 gap-2">
        {APP_LANGUAGES.map((l: AppLanguage) => (
          <button
            key={l}
            type="button"
            lang={l}
            aria-pressed={language === l}
            onClick={() => useSettingsStore.getState().update({ language: l })}
            className={cx(
              "min-h-target rounded-2xl border px-4 text-base font-semibold transition-colors duration-ui",
              language === l ? "border-brand bg-brand-soft text-foreground" : "border-separator bg-card text-muted-foreground",
            )}
          >
            {t(`language.native.${l}`)}
          </button>
        ))}
      </div>
    </div>
  );
}

export function ProfileStep({ onNext }: StepProps) {
  const { t } = useT("onboarding");
  const { t: tc } = useT("common");
  const settings = useSettingsStore();
  const [name, setName] = useState(settings.displayName);
  const done = name.trim() !== "";
  const VehicleIcon = settings.vehicle === "moto" ? Bike : Car;
  return (
    <form
      className="flex flex-col gap-6"
      onSubmit={(e) => {
        e.preventDefault();
        if (!done) return;
        useSettingsStore.getState().setDisplayName(name);
        onNext();
      }}
    >
      <LanguagePicker />
      <NameField value={name} onChange={setName} autoFocus />
      <div className="flex flex-col gap-2">
        <span className="text-sm font-semibold">{t("profile.vehicle")}</span>
        <SegmentedControl<Vehicle>
          label={t("profile.vehicle")}
          value={settings.vehicle}
          onChange={(v) => useSettingsStore.getState().setVehicle(v)}
          options={[
            { value: "moto", label: tc("vehicle.moto") },
            { value: "car", label: tc("vehicle.car") },
          ]}
        />
        <p className="flex items-start gap-2 rounded-2xl bg-card p-3 text-sm text-muted-foreground">
          <VehicleIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-brand" />
          {t(`profile.defaults.${settings.vehicle}`)}
        </p>
      </div>
      <Button type="submit" variant="primary" size="lg" block disabled={!done}>
        {tc("action.continue")}
      </Button>
    </form>
  );
}

const PERMISSION_ICONS: Readonly<Record<string, LucideIcon>> = { microphone: Mic, bluetooth: Bluetooth, notifications: Bell };

function stateText(state: string | undefined, t: (k: string) => string): string {
  if (state === "granted") return t("permissions.granted");
  if (state === "denied") return t("permissions.denied");
  return t("permissions.notAsked");
}

export function PermissionsStep({ onNext }: StepProps) {
  const { t: tc } = useT("common");
  const { t } = useT("onboarding");
  const engine = useIntercomEngine();
  const [status, setStatus] = useState<Partial<IntercomPermissionStatus>>({});
  const [asking, setAsking] = useState(false);
  const [asked, setAsked] = useState(false);

  useEffect(() => {
    let alive = true;
    engine.checkPermissions().then(
      (s) => {
        if (alive) setStatus(s);
      },
      (e: unknown) => console.warn("[onboarding] checkPermissions failed", e),
    );
    return () => {
      alive = false;
    };
  }, [engine]);

  const ask = () => {
    setAsking(true);
    engine
      .requestPermissions([...ONBOARDING_PERMISSIONS])
      .then(
        (s) => {
          setStatus(s);
          setAsked(true);
        },
        (e: unknown) => {
          console.warn("[onboarding] requestPermissions failed", e);
          setAsked(true);
        },
      )
      .finally(() => setAsking(false));
  };

  const anyDenied = ONBOARDING_PERMISSIONS.some((p) => status[p] === "denied");
  return (
    <div className="flex flex-col gap-5">
      <p className="text-base text-muted-foreground">{t("permissions.intro")}</p>
      <ul className="flex flex-col divide-y divide-separator rounded-section border border-separator bg-card text-card-foreground shadow-surface" data-testid="permission-list">
        {ONBOARDING_PERMISSIONS.map((p) => {
          const Icon = PERMISSION_ICONS[p] ?? Mic;
          const granted = status[p] === "granted";
          return (
            <li key={p} className="flex items-start gap-3 p-4" data-permission={p} data-state={status[p] ?? "prompt"}>
              <span aria-hidden="true" className="grid size-10 shrink-0 place-items-center rounded-full bg-brand-soft text-brand">
                <Icon className="size-5" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center justify-between gap-x-3">
                  <span className="font-semibold">{t(`permissions.${p}.title`)}</span>
                  <span className={cx("inline-flex items-center gap-1 text-sm", granted ? "text-talk-active" : "text-muted-foreground")}>
                    {granted && <Check aria-hidden="true" className="size-4" strokeWidth={3} />}
                    {stateText(status[p], t)}
                  </span>
                </div>
                <p className="mt-0.5 text-sm text-muted-foreground">{t(`permissions.${p}.reason`)}</p>
              </div>
            </li>
          );
        })}
      </ul>
      {anyDenied && <p className="text-sm text-muted-foreground">{t("permissions.deniedHint")}</p>}
      <DistanceCard />
      {asked ? (
        <Button variant="primary" size="lg" block onClick={onNext}>
          {tc("action.continue")}
        </Button>
      ) : (
        <div className="flex flex-col gap-1">
          <Button variant="primary" size="lg" block disabled={asking} onClick={ask}>
            {t("permissions.allow")}
          </Button>
          <Button variant="plain" block onClick={onNext}>
            {t("permissions.later")}
          </Button>
        </div>
      )}
    </div>
  );
}

/** "Share my distance" inside the permissions step: location is asked only from this tap. */
function DistanceCard() {
  const { t } = useT("onboarding");
  const engine = useIntercomEngine();
  const sharing = useSettingsStore((s) => s.shareDistance);
  const [asking, setAsking] = useState(false);
  const [denied, setDenied] = useState(false);

  const share = () => {
    setAsking(true);
    engine
      .requestPermissions(["location"])
      .then(
        (s) => {
          const granted = s.location === "granted";
          useSettingsStore.getState().update({ shareDistance: granted });
          setDenied(!granted);
        },
        (e: unknown) => {
          console.warn("[onboarding] location permission failed", e);
          setDenied(true);
        },
      )
      .finally(() => setAsking(false));
  };

  return (
    <section className="flex flex-col gap-3 rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface" data-testid="distance-card">
      <div className="flex items-start gap-3">
        <span aria-hidden="true" className="grid size-10 shrink-0 place-items-center rounded-full bg-brand-soft text-brand">
          <MapPin className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="font-semibold">{t("distance.question")}</h3>
          <p className="mt-0.5 text-sm text-muted-foreground">{t("distance.what")}</p>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">{t("distance.keepAlive")} {t("distance.privacy")}</p>
      {denied && <p className="text-sm text-danger">{t("distance.denied")}</p>}
      {sharing ? (
        <p className="inline-flex min-h-target items-center gap-1.5 text-sm font-semibold text-talk-active">
          <Check aria-hidden="true" className="size-4" strokeWidth={3} />
          {t("permissions.granted")}
        </p>
      ) : (
        <Button variant="secondary" block disabled={asking} onClick={share}>
          {t("distance.yes")}
        </Button>
      )}
    </section>
  );
}
