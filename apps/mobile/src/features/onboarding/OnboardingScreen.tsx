// Onboarding (WP-22, DESIGN §1.3 F0, §11.4): language → name + vehicle → permissions →
// Share my distance → headset check (skippable) → "don't swipe the app away" and the
// honest range promise. About 90 s, all in the foreground.
import { Button } from "@meshapp/ui-kit";
import { useState } from "react";

import type { ScreenProps } from "../../app/router";
import { SCREEN, useRouter } from "../../app/router";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useBackHandler } from "../../components/useBackHandler";
import { useT } from "../../i18n/index";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { syncEngineSettings } from "../group/session";
import { HeadsetStep } from "./HeadsetStep";
import { DistanceStep, LanguageStep, PermissionsStep, ProfileStep } from "./steps";

export const ONBOARDING_STEPS = ["language", "profile", "permissions", "distance", "headset", "ready"] as const;
export type OnboardingStep = (typeof ONBOARDING_STEPS)[number];

function ReadyStep({ onFinish }: { readonly onFinish: () => void }) {
  const { t } = useT("onboarding");
  return (
    <div className="flex flex-col gap-4" data-testid="ready-step">
      <section className="rounded-lg border-s-4 border-ladder-warn bg-card p-4 text-card-foreground">
        <h3 className="text-lg font-semibold">{t("ready.noSwipeTitle")}</h3>
        <p className="mt-1 text-sm text-muted-foreground">{t("ready.noSwipe")}</p>
      </section>
      <section className="rounded-lg bg-card p-4 text-card-foreground" data-testid="honest-promise">
        <h3 className="text-lg font-semibold">{t("promise.title")}</h3>
        <ul className="mt-2 flex list-inside list-disc flex-col gap-1.5 text-sm">
          <li>{t("promise.phones")}</li>
          <li>{t("promise.range")}</li>
          <li>{t("promise.relays")}</li>
          <li>{t("promise.notHeadset")}</li>
        </ul>
      </section>
      <p className="text-sm text-muted-foreground">{t("ready.safety")}</p>
      <Button variant="primary" size="lg" block onClick={onFinish} data-testid="onboarding-finish">
        {t("ready.finish")}
      </Button>
    </div>
  );
}

export function OnboardingScreen(_props: ScreenProps) {
  const { t } = useT("onboarding");
  const engine = useIntercomEngine();
  const [index, setIndex] = useState(0);
  const step: OnboardingStep = ONBOARDING_STEPS[index] ?? "ready";
  const next = () => setIndex((i) => Math.min(i + 1, ONBOARDING_STEPS.length - 1));
  const back = () => {
    if (index > 0) setIndex(index - 1);
    else useRouter.getState().back();
  };
  useBackHandler(() => {
    back();
    return true;
  });

  const finish = () => {
    useSettingsStore.getState().update({ onboardingDone: true });
    // The engine learns the name and vehicle now; refusals only mean it is not up yet.
    void syncEngineSettings(engine);
    useRouter.getState().reset(SCREEN.home);
  };

  return (
    <ScreenLayout title={t(`step.${step}`)} back onBack={back}>
      <div className="mx-auto flex max-w-xl flex-col gap-5 pb-6" data-testid="onboarding" data-step={step}>
        <div className="flex flex-col gap-2">
          <p className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
            {t("progress", { n: index + 1, total: ONBOARDING_STEPS.length })}
          </p>
          <div
            className="h-1.5 w-full overflow-hidden rounded-full bg-control-fill"
            role="progressbar"
            aria-label={t("progressLabel")}
            aria-valuemin={1}
            aria-valuemax={ONBOARDING_STEPS.length}
            aria-valuenow={index + 1}
          >
            <div className="h-full rounded-full bg-brand" style={{ width: `${((index + 1) / ONBOARDING_STEPS.length) * 100}%` }} />
          </div>
        </div>
        {step === "language" && <LanguageStep onNext={next} />}
        {step === "profile" && <ProfileStep onNext={next} />}
        {step === "permissions" && <PermissionsStep onNext={next} />}
        {step === "distance" && <DistanceStep onNext={next} />}
        {step === "headset" && <HeadsetStep onNext={next} />}
        {step === "ready" && <ReadyStep onFinish={finish} />}
      </div>
    </ScreenLayout>
  );
}
