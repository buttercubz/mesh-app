// Shared frame of every onboarding step: the screen title, the back button, the step
// progress and a pinned footer with the step's primary action. Each step renders its own
// frame so its action lives in the ScreenLayout footer and stays on screen on short
// phones and with large text (the body scrolls above it).
import { cx } from "@meshapp/ui-kit";
import type { ReactNode } from "react";

import { ScreenLayout } from "../../components/ScreenLayout";
import { useT } from "../../i18n/index";

export const ONBOARDING_STEPS = ["profile", "permissions", "headset", "ready"] as const;
export type OnboardingStep = (typeof ONBOARDING_STEPS)[number];

/** Where the rider is in the walkthrough, passed from OnboardingScreen to each step. */
export interface OnboardingPosition {
  readonly step: OnboardingStep;
  readonly index: number;
  readonly onBack: () => void;
}

export interface OnboardingFrameProps {
  readonly position: OnboardingPosition;
  /** The step's primary action (and its plain alternative), pinned under the body. */
  readonly footer: ReactNode;
  readonly children: ReactNode;
}

export function OnboardingFrame({ position, footer, children }: OnboardingFrameProps) {
  const { t } = useT("onboarding");
  const { step, index, onBack } = position;
  return (
    <ScreenLayout title={t(`step.${step}`)} back onBack={onBack} footer={<div className="mx-auto flex max-w-xl flex-col gap-1">{footer}</div>}>
      <div className="mx-auto flex max-w-xl flex-col gap-6 pb-2" data-testid="onboarding" data-step={step}>
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
        {children}
      </div>
    </ScreenLayout>
  );
}
