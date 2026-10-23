// Onboarding steps 1-4 (WP-22, DESIGN §1.3 F0): language, name + vehicle, permission
// walkthrough (each with a one-line reason, all in the foreground) and the "Share my
// distance" choice with its keep-alive explanation.
import type { Vehicle } from "@meshapp/engine-api";
import { Button, SegmentedControl } from "@meshapp/ui-kit";
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

export function LanguageStep({ onNext }: StepProps) {
  const { t } = useT("onboarding");
  const language = useSettingsStore((s) => s.language);
  const pick = (l: AppLanguage) => {
    useSettingsStore.getState().update({ language: l });
    onNext();
  };
  return (
    <div className="flex flex-col gap-4">
      <p className="text-lg">{t("language.question")}</p>
      {APP_LANGUAGES.map((l) => (
        <Button key={l} variant={language === l ? "primary" : "secondary"} size="lg" block lang={l} onClick={() => pick(l)}>
          {t(`language.native.${l}`)}
        </Button>
      ))}
    </div>
  );
}

export function ProfileStep({ onNext }: StepProps) {
  const { t } = useT("onboarding");
  const { t: tc } = useT("common");
  const settings = useSettingsStore();
  const [name, setName] = useState(settings.displayName);
  const done = name.trim() !== "";
  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(e) => {
        e.preventDefault();
        if (!done) return;
        useSettingsStore.getState().setDisplayName(name);
        onNext();
      }}
    >
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
        <p className="text-sm text-muted-foreground">{t(`profile.defaults.${settings.vehicle}`)}</p>
      </div>
      <Button type="submit" variant="primary" size="lg" block disabled={!done}>
        {tc("action.continue")}
      </Button>
    </form>
  );
}

function stateText(state: string | undefined, t: (k: string) => string): string {
  if (state === "granted") return t("permissions.granted");
  if (state === "denied") return t("permissions.denied");
  return t("permissions.notAsked");
}

export function PermissionsStep({ onNext }: StepProps) {
  const { t } = useT("onboarding");
  const { t: tc } = useT("common");
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
    <div className="flex flex-col gap-4">
      <p className="text-base">{t("permissions.intro")}</p>
      <ul className="flex flex-col gap-3" data-testid="permission-list">
        {ONBOARDING_PERMISSIONS.map((p) => (
          <li key={p} className="rounded-lg bg-card p-3 text-card-foreground" data-permission={p} data-state={status[p] ?? "prompt"}>
            <div className="flex items-center justify-between gap-3">
              <span className="font-semibold">{t(`permissions.${p}.title`)}</span>
              <span className={status[p] === "granted" ? "text-sm text-talk-active" : "text-sm text-muted-foreground"}>
                {stateText(status[p], t)}
              </span>
            </div>
            <p className="mt-1 text-sm text-muted-foreground">{t(`permissions.${p}.reason`)}</p>
          </li>
        ))}
      </ul>
      {anyDenied && <p className="text-sm text-muted-foreground">{t("permissions.deniedHint")}</p>}
      {asked ? (
        <Button variant="primary" size="lg" block onClick={onNext}>
          {tc("action.continue")}
        </Button>
      ) : (
        <>
          <Button variant="primary" size="lg" block disabled={asking} onClick={ask}>
            {t("permissions.allow")}
          </Button>
          <Button variant="plain" block onClick={onNext}>
            {t("permissions.later")}
          </Button>
        </>
      )}
    </div>
  );
}

export function DistanceStep({ onNext }: StepProps) {
  const { t } = useT("onboarding");
  const engine = useIntercomEngine();
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
          if (granted) onNext();
          else setDenied(true);
        },
        (e: unknown) => {
          console.warn("[onboarding] location permission failed", e);
          setDenied(true);
        },
      )
      .finally(() => setAsking(false));
  };

  return (
    <div className="flex flex-col gap-4">
      <p className="text-lg font-semibold">{t("distance.question")}</p>
      <p className="text-base">{t("distance.what")}</p>
      <p className="text-sm text-muted-foreground">{t("distance.keepAlive")}</p>
      <p className="text-sm text-muted-foreground">{t("distance.privacy")}</p>
      {denied && <p className="text-sm text-danger">{t("distance.denied")}</p>}
      <Button variant="primary" size="lg" block disabled={asking} onClick={share}>
        {t("distance.yes")}
      </Button>
      <Button
        variant="secondary"
        size="lg"
        block
        onClick={() => {
          useSettingsStore.getState().update({ shareDistance: false });
          onNext();
        }}
      >
        {t("distance.no")}
      </Button>
    </div>
  );
}
