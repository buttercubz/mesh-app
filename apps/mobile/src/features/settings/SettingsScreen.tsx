// Settings (WP-22, DESIGN §11.4): profile (name, vehicle), talk and audio, language,
// theme and spoken prompts, Share my distance, Field test (visible, plain copy),
// Diagnostics (ridelog/1 recorder) and about with the honest range promise.
import type { Vehicle } from "@meshapp/engine-api";
import type { ThemePreference } from "@meshapp/ui-kit";
import { Avatar, Button, ListSection, SegmentedControl } from "@meshapp/ui-kit";
import { Bike, Car, RotateCcw } from "lucide-react";
import { useState } from "react";

import type { ScreenProps } from "../../app/router";
import { useRouter } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import type { AppLanguage } from "../../i18n/index";
import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { clampDisplayName, useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { NameField } from "../group/NameField";
import { runCommand } from "../group/session";
import { FieldTestRow } from "../lab/entry";
import { ONBOARDING_SCREEN } from "../onboarding/ids";
import { AwareSection } from "./AwareSection";
import { SettingRow, SettingsGroup, Switch } from "./controls";
import { DiagnosticsSection } from "./DiagnosticsSection";
import { TalkSettings } from "./TalkSettings";

/** Who this phone is to the group: avatar, name and vehicle at a glance. */
function ProfileHeader() {
  const { t: tc } = useT("common");
  const name = useSettingsStore((s) => s.displayName);
  const vehicle = useSettingsStore((s) => s.vehicle);
  const VehicleIcon = vehicle === "car" ? Car : Bike;
  return (
    <div className="flex items-center gap-4 px-1 pt-2" data-testid="settings-profile">
      <Avatar name={name.trim() === "" ? "?" : name} size={64} />
      <div className="min-w-0">
        <p className="truncate text-2xl font-bold tracking-tight">{name}</p>
        <p className="inline-flex items-center gap-1.5 text-sm text-muted-foreground">
          <VehicleIcon aria-hidden="true" className="size-4" />
          {tc(`vehicle.${vehicle}`)}
        </p>
      </div>
    </div>
  );
}

function ProfileSettings() {
  const { t } = useT("settings");
  const { t: tc } = useT("common");
  const engine = useIntercomEngine();
  const saved = useSettingsStore((s) => s.displayName);
  const vehicle = useSettingsStore((s) => s.vehicle);
  const [name, setName] = useState(saved);
  const commitName = () => {
    const clean = clampDisplayName(name);
    if (clean === "" || clean === saved) return;
    useSettingsStore.getState().setDisplayName(clean);
    void runCommand(engine, { type: "setDisplayName", name: clean }, true);
  };
  return (
    <SettingsGroup title={t("profile.title")}>
      <div className="px-4 py-3" onBlur={commitName}>
        <NameField value={name} onChange={setName} />
      </div>
      <SettingRow label={t("profile.vehicle")} description={t(`profile.vehicleHint.${vehicle}`)} stacked>
        <SegmentedControl<Vehicle>
          label={t("profile.vehicle")}
          value={vehicle}
          onChange={(v) => {
            useSettingsStore.getState().setVehicle(v);
            void runCommand(engine, { type: "setVehicle", vehicle: v }, true);
          }}
          options={[
            { value: "moto", label: tc("vehicle.moto") },
            { value: "car", label: tc("vehicle.car") },
          ]}
        />
      </SettingRow>
    </SettingsGroup>
  );
}

function DisplaySettings() {
  const { t } = useT("settings");
  const { t: tc } = useT("common");
  const language = useSettingsStore((s) => s.language);
  const theme = useSettingsStore((s) => s.theme);
  const prompts = useSettingsStore((s) => s.spokenPrompts);
  const update = useSettingsStore((s) => s.update);
  return (
    <SettingsGroup title={t("display.title")}>
      <SettingRow label={t("display.language")} stacked>
        <SegmentedControl<AppLanguage>
          label={t("display.language")}
          value={language}
          onChange={(l) => update({ language: l })}
          options={[
            { value: "es", label: "Español" },
            { value: "en", label: "English" },
          ]}
        />
      </SettingRow>
      <SettingRow label={t("display.theme")} description={t("display.themeHint")} stacked>
        <SegmentedControl<ThemePreference>
          label={t("display.theme")}
          value={theme}
          onChange={(v) => update({ theme: v })}
          options={(["dark", "light", "system"] as const).map((v) => ({ value: v, label: tc(`theme.${v}`) }))}
        />
      </SettingRow>
      <SettingRow label={t("display.prompts")} description={t("display.promptsHint")}>
        <Switch label={t("display.prompts")} checked={prompts} onChange={(on) => update({ spokenPrompts: on })} />
      </SettingRow>
    </SettingsGroup>
  );
}

function PrivacySettings() {
  const { t } = useT("settings");
  const engine = useIntercomEngine();
  const share = useSettingsStore((s) => s.shareDistance);
  const [denied, setDenied] = useState(false);
  const set = (enabled: boolean) => {
    if (!enabled) {
      useSettingsStore.getState().update({ shareDistance: false });
      void runCommand(engine, { type: "setShareDistance", enabled: false }, true);
      return;
    }
    // Location is asked only when the rider turns sharing on (foreground, from this tap).
    engine.requestPermissions(["location"]).then(
      (s) => {
        const granted = s.location === "granted";
        setDenied(!granted);
        if (!granted) return;
        useSettingsStore.getState().update({ shareDistance: true });
        void runCommand(engine, { type: "setShareDistance", enabled: true }, true);
      },
      (e: unknown) => {
        console.warn("[settings] location permission failed", e);
        setDenied(true);
      },
    );
  };
  return (
    <SettingsGroup title={t("privacy.title")}>
      <SettingRow label={t("privacy.shareDistance")} description={t("privacy.shareDistanceHint")}>
        <Switch label={t("privacy.shareDistance")} checked={share} onChange={set} />
      </SettingRow>
      {denied && <p className="px-4 pb-3 text-sm text-danger">{t("privacy.locationDenied")}</p>}
    </SettingsGroup>
  );
}

function AboutSettings() {
  const { t } = useT("settings");
  const { t: tc } = useT("common");
  const info = useEngineStore((s) => s.info);
  return (
    <SettingsGroup title={t("about.title")}>
      <div className="flex flex-col gap-2 px-4 py-3 text-sm">
        <p>
          {tc("appName")} {info?.appVersion ?? ""}
        </p>
        {info !== null && (
          <p className="text-muted-foreground">
            {t("about.engine", { platform: info.platform, api: info.apiVersion, protocol: info.protocolVersion })}
          </p>
        )}
        <p className="leading-relaxed text-muted-foreground">{tc("shell.promise")}</p>
        <p className="text-muted-foreground">{t("about.noChat")}</p>
      </div>
      <div className="px-4 py-3">
        <Button variant="secondary" block onClick={() => useRouter.getState().push(ONBOARDING_SCREEN)}>
          <RotateCcw aria-hidden="true" />
          {t("about.setupAgain")}
        </Button>
      </div>
    </SettingsGroup>
  );
}

export function SettingsScreen(_props: ScreenProps) {
  const { t } = useT("settings");
  return (
    <ScreenLayout title={t("title")}>
      <div className="mx-auto flex max-w-xl flex-col gap-7 pb-8" data-testid="settings-screen">
        <ProfileHeader />
        <ProfileSettings />
        <TalkSettings />
        <DisplaySettings />
        <PrivacySettings />
        <section className="flex flex-col gap-2">
          <h2 className="px-4 text-xs font-semibold tracking-wider text-muted-foreground uppercase">{t("fieldTest.title")}</h2>
          <ListSection>
            <FieldTestRow />
          </ListSection>
        </section>
        <AwareSection />
        <DiagnosticsSection />
        <AboutSettings />
      </div>
    </ScreenLayout>
  );
}
