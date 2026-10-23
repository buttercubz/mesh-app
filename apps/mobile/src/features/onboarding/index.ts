// Onboarding feature (WP-22, DESIGN §1.3 F0, §11.4). Loaded by app/features.ts.
import { registerScreen } from "../../app/router";
import { ONBOARDING_SCREEN } from "./ids";
import { OnboardingScreen } from "./OnboardingScreen";

registerScreen(ONBOARDING_SCREEN, OnboardingScreen);

export { OnboardingScreen } from "./OnboardingScreen";
