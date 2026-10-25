// Onboarding (WP-22, DESIGN §1.3 F0, §11.4), collapsed to the essentials: about you
// (language, name, vehicle) → permissions with Share my distance → headset check
// (skippable) → "don't swipe the app away" and the honest range promise. About a
// minute, all in the foreground.
import { Button, cx } from "@meshapp/ui-kit";
import { Hand, Smartphone } from "lucide-react";
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
import { PermissionsStep, ProfileStep } from "./steps";

export const ONBOARDING_STEPS = ["profile", "permissions", "headset", "ready"] as const;
export type OnboardingStep = (typeof ONBOARDING_STEPS)[number];

function ReadyStep({ onFinish }: { readonly onFinish: () => void }) {
  const { t } = useT("onboarding");
  return (
    <div className="flex flex-col gap-4" data-testid="ready-step">
      <section className="flex items-start gap-3 rounded-section border border-ladder-warn/40 bg-ladder-warn/10 p-4">
        <span aria-hidden="true" className="grid size-10 shrink-0 place-items-center rounded-full bg-ladder-warn/20">
          <Hand className="size-5" />
        </span>
        <div>
          <h3 className="text-base font-semibold">{t("ready.noSwipeTitle")}</h3>
          <p className="mt-1 text-sm text-muted-foreground">{t("ready.noSwipe")}</p>
        </div>
      </section>
      <section className="rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface" data-testid="honest-promise">
        <h3 className="flex items-center gap-2 text-base font-semibold">
          <Smartphone aria-hidden="true" className="size-5 text-brand" />
          {t("promise.title")}
        </h3>
        <ul className="mt-3 flex list-disc flex-col gap-2 ps-5 text-sm marker:text-brand">
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
      <div className="mx-auto flex max-w-xl flex-col gap-6 pb-6" data-testid="onboarding" data-step={step}>
        <div className="flex flex-col gap-2">
          <p className="text-xs font-semibold tracking-wider text-muted-foreground uppercase">
            {t("progress", { n: index + 1, total: ONBOARDING_STEPS.length })}
          </p>
          <div
            className="flex gap-1.5"
            role="progressbar"
            aria-label={t("progressLabel")}
            aria-valuemin={1}
            aria-valuemax={ONBOARDING_STEPS.length}
            aria-valuenow={index + 1}
          >
            {ONBOARDING_STEPS.map((s, i) => (
              <span
                key={s}
                className={cx("h-1.5 flex-1 rounded-full transition-colors duration-ui", i <= index ? "bg-brand" : "bg-control-fill")}
              />
            ))}
          </div>
        </div>
        {step === "profile" && <ProfileStep onNext={next} />}
        {step === "permissions" && <PermissionsStep onNext={next} />}
        {step === "headset" && <HeadsetStep onNext={next} />}
        {step === "ready" && <ReadyStep onFinish={finish} />}
      </div>
    </ScreenLayout>
  );
}
