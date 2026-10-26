// Onboarding (WP-22, DESIGN §1.3 F0, §11.4), collapsed to the essentials: about you
// (language, name, vehicle) → permissions with Share my distance → headset check
// (skippable) → "don't swipe the app away" and the honest range promise. About a
// minute, all in the foreground.
import { Button } from "@meshapp/ui-kit";
import { Hand, Smartphone } from "lucide-react";
import { useState } from "react";

import type { ScreenProps } from "../../app/router";
import { SCREEN, useRouter } from "../../app/router";
import { useBackHandler } from "../../components/useBackHandler";
import { useT } from "../../i18n/index";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { syncEngineSettings } from "../group/session";
import { HeadsetStep } from "./HeadsetStep";
import type { OnboardingPosition, OnboardingStep } from "./OnboardingFrame";
import { ONBOARDING_STEPS, OnboardingFrame } from "./OnboardingFrame";
import { PermissionsStep, ProfileStep } from "./steps";

export { ONBOARDING_STEPS } from "./OnboardingFrame";
export type { OnboardingStep } from "./OnboardingFrame";

function ReadyStep({ position, onFinish }: { readonly position: OnboardingPosition; readonly onFinish: () => void }) {
  const { t } = useT("onboarding");
  return (
    <OnboardingFrame
      position={position}
      footer={
        <Button variant="primary" size="lg" block onClick={onFinish} data-testid="onboarding-finish">
          {t("ready.finish")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4" data-testid="ready-step">
        <section className="flex items-start gap-3 rounded-section border border-ladder-warn/40 bg-ladder-warn/10 p-4">
          <span aria-hidden="true" className="grid size-10 shrink-0 place-items-center rounded-full bg-ladder-warn/20">
            <Hand className="size-5" />
          </span>
          <div className="min-w-0">
            <h3 className="text-base font-semibold text-pretty">{t("ready.noSwipeTitle")}</h3>
            <p className="mt-1 text-sm text-muted-foreground">{t("ready.noSwipe")}</p>
          </div>
        </section>
        <section className="rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface" data-testid="honest-promise">
          <h3 className="flex items-center gap-2 text-base font-semibold">
            <Smartphone aria-hidden="true" className="size-5 shrink-0 text-brand" />
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
      </div>
    </OnboardingFrame>
  );
}

export function OnboardingScreen(_props: ScreenProps) {
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

  const position: OnboardingPosition = { step, index, onBack: back };
  // Each step renders its own frame (title, progress, pinned primary action).
  if (step === "profile") return <ProfileStep position={position} onNext={next} />;
  if (step === "permissions") return <PermissionsStep position={position} onNext={next} />;
  if (step === "headset") return <HeadsetStep position={position} onNext={next} />;
  return <ReadyStep position={position} onFinish={finish} />;
}
