// @vitest-environment jsdom
// Onboarding (WP-22 acceptance): language, 24-byte name, vehicle defaults, permissions
// in the foreground, Share my distance with location, the headset check through
// gestureCommand(audioCheck) rendering the audioCheckSample fixture, the "don't swipe"
// and honest-promise step.
import { fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, onTestFinished } from "vitest";

import { installSettingsEffects } from "../../app/settingsEffects";
import { useRouter } from "../../app/router";
import { useSettingsStore } from "../../store/useSettingsStore";
import { calls, emit, fixtureEvent, mountWith, resetAll, settle } from "../group/testing";
import { OnboardingScreen } from "./OnboardingScreen";

afterEach(resetAll);

const step = () => screen.getByTestId("onboarding").getAttribute("data-step");

describe("OnboardingScreen", () => {
  it("walks through every step and finishes on Home", async () => {
    useSettingsStore.setState({ onboardingDone: false, displayName: "" });
    // The shell applies the language on change (settingsEffects); prompts are not synced here.
    const stopEffects = installSettingsEffects({ syncPrompts: () => Promise.resolve({ ok: true }) });
    onTestFinished(stopEffects);
    const { plugin } = mountWith(<OnboardingScreen params={{}} />);
    await settle();
    expect(step()).toBe("language");
    fireEvent.click(screen.getByRole("button", { name: "Español" }));
    expect(useSettingsStore.getState().language).toBe("es");
    expect(step()).toBe("profile");
    expect(screen.getByText("Paso 2 de 6")).toBeTruthy();
    // Back to English for the rest of the assertions.
    fireEvent.click(screen.getByRole("button", { name: "Atrás" }));
    fireEvent.click(screen.getByRole("button", { name: "English" }));

    // Name: limited to 24 UTF-8 bytes at a grapheme boundary.
    const name = screen.getByLabelText("Your name");
    fireEvent.change(name, { target: { value: "Ñandú García Rodríguez Pérez" } });
    expect(new TextEncoder().encode((name as HTMLInputElement).value).length).toBeLessThanOrEqual(24);
    fireEvent.click(screen.getByRole("radio", { name: "Car" }));
    expect(useSettingsStore.getState()).toMatchObject({ vehicle: "car", defaultMode: "ptt", keepScreenOnMounted: true });
    expect(screen.getByText(/Car: push to talk/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(useSettingsStore.getState().displayName.startsWith("Ñandú")).toBe(true);

    expect(step()).toBe("permissions");
    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await settle();
    expect(calls(plugin, "requestPermissions")).toEqual([{ permissions: ["microphone", "bluetooth", "notifications"] }]);
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(step()).toBe("distance");
    expect(screen.getByText(/keeps the intercom running/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Share my distance" }));
    await settle();
    // The fake plugin denies location: sharing stays off and the screen says why.
    expect(calls(plugin, "requestPermissions").at(-1)).toEqual({ permissions: ["location"] });
    expect(useSettingsStore.getState().shareDistance).toBe(false);
    expect(screen.getByText(/Location was not allowed/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));

    expect(step()).toBe("headset");
    fireEvent.click(screen.getByTestId("check-tone"));
    expect(plugin.gestured).toEqual([{ type: "audioCheck", op: "start", mode: "tone" }]);
    expect(plugin.sent.some((c) => c.type === "audioCheck" && c.op === "start")).toBe(false);
    emit(fixtureEvent("audioCheckSample"));
    expect(screen.getByTestId("meter-route").textContent).toBe("Headset · HD voice · 16 kHz");
    expect(screen.getByTestId("meter-gate").textContent).toBe("Voice detected");
    expect(screen.getByRole("meter", { name: "Microphone level" }).getAttribute("aria-valuenow")).toBe("35");
    expect(screen.getByText("Recording, say something")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(plugin.sent).toContainEqual({ type: "audioCheck", op: "stop" });

    expect(step()).toBe("ready");
    expect(screen.getByText("Do not swipe the app away during a ride")).toBeTruthy();
    expect(screen.getByTestId("honest-promise").textContent).toContain("clear voice up to 2 relays, further is best effort");
    fireEvent.click(screen.getByTestId("onboarding-finish"));
    await settle();
    expect(useSettingsStore.getState().onboardingDone).toBe(true);
    expect(useRouter.getState().stack.map((r) => r.id)).toEqual(["home"]);
    expect(plugin.sent).toContainEqual({ type: "setVehicle", vehicle: "car" });
  });

  it("the headset check is skippable and stops a running check on unmount", async () => {
    const { plugin, unmount } = mountWith(<OnboardingScreen params={{}} />);
    fireEvent.click(screen.getByRole("button", { name: "English" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(step()).toBe("headset");
    fireEvent.click(screen.getByTestId("check-echo"));
    expect(plugin.gestured).toEqual([{ type: "audioCheck", op: "start", mode: "echo" }]);
    unmount();
    await settle();
    expect(plugin.sent).toContainEqual({ type: "audioCheck", op: "stop" });
  });
});
