// @vitest-environment jsdom
// Settings (WP-22 acceptance): the VOX test meter through gestureCommand(audioCheck
// voxMeter) rendering the audioCheckSample fixture; settings saved and sent to the
// engine; car-only "keep screen on while mounted"; Share my distance asks for location;
// Diagnostics (ridelog/1): usage, toggle, export, fixed mark chips, confirmed clear.
import { fireEvent, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "../../store/useSettingsStore";
import { calls, emit, fixtureEvent, mountWith, resetAll, settle } from "../group/testing";
import { SettingsScreen } from "./SettingsScreen";

afterEach(resetAll);

describe("SettingsScreen", () => {
  it("runs the VOX test meter from the tap and renders the samples", async () => {
    const { plugin } = mountWith(<SettingsScreen params={{}} />);
    fireEvent.click(screen.getByTestId("vox-test"));
    expect(plugin.gestured).toEqual([{ type: "audioCheck", op: "start", mode: "voxMeter" }]);
    emit(fixtureEvent("audioCheckSample"));
    expect(screen.getByRole("meter", { name: "VOX test level" }).getAttribute("aria-valuenow")).toBe("35");
    expect(screen.getByTestId("meter-gate").textContent).toBe("Voice detected");
    expect(screen.getByTestId("meter-route").textContent).toBe("Headset · HD voice · 16 kHz");
    fireEvent.click(screen.getByTestId("vox-test"));
    await settle();
    expect(plugin.sent).toContainEqual({ type: "audioCheck", op: "stop" });
    expect(screen.queryByRole("meter", { name: "VOX test level" })).toBeNull();
  });

  it("saves talk settings and sends them to the engine", async () => {
    const { plugin } = mountWith(<SettingsScreen params={{}} />);
    fireEvent.click(screen.getByRole("radio", { name: "Latch" }));
    fireEvent.click(screen.getByRole("radio", { name: "High" }));
    fireEvent.click(screen.getByRole("switch", { name: "Talk with the phone mic" }));
    await settle();
    expect(useSettingsStore.getState()).toMatchObject({ pttStyle: "latch", voxSensitivity: "high", builtinMicTx: true });
    expect(plugin.sent).toEqual(
      expect.arrayContaining([
        { type: "setPttStyle", style: "latch" },
        { type: "setVoxSensitivity", level: "high" },
        { type: "setBuiltinMicTx", allowed: true },
      ]),
    );
  });

  it("offers keep-screen-on while mounted only for the car, on by default", async () => {
    const { plugin } = mountWith(<SettingsScreen params={{}} />);
    expect(screen.queryByTestId("keep-screen-on-row")).toBeNull();
    fireEvent.click(screen.getAllByRole("radio", { name: "Car" })[0] as HTMLElement);
    await settle();
    expect(plugin.sent).toContainEqual({ type: "setVehicle", vehicle: "car" });
    const toggle = screen.getByRole("switch", { name: "Keep screen on while mounted" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(toggle);
    expect(useSettingsStore.getState().keepScreenOnMounted).toBe(false);
  });

  it("Share my distance asks for location and stays off when it is denied", async () => {
    const { plugin } = mountWith(<SettingsScreen params={{}} />);
    fireEvent.click(screen.getByRole("switch", { name: "Share my distance" }));
    await settle();
    expect(calls(plugin, "requestPermissions")).toEqual([{ permissions: ["location"] }]);
    expect(useSettingsStore.getState().shareDistance).toBe(false);
    expect(screen.getByText(/Location is not allowed/)).toBeTruthy();
  });

  it("Diagnostics: shows usage, toggles, exports, marks with fixed chips and clears after confirming", async () => {
    const { plugin } = mountWith(<SettingsScreen params={{}} />);
    await settle();
    expect(calls(plugin, "diagStatus").length).toBeGreaterThan(0);
    expect(screen.getByTestId("diag-usage").textContent).toBe("Using 12.4 MB · 7 sessions · oldest 3 days ago");
    const toggle = screen.getByRole("switch", { name: "Record diagnostics" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(toggle);
    await settle();
    expect(calls(plugin, "diagSetEnabled")).toEqual([{ enabled: false }]);
    fireEvent.click(screen.getByRole("button", { name: "Export last ride" }));
    await settle();
    expect(calls(plugin, "diagExport")).toEqual([{ scope: "last" }]);
    expect(screen.getByRole("status").textContent).toBe("Diagnostics exported.");
    fireEvent.click(screen.getByRole("button", { name: "Export all" }));
    await settle();
    expect(calls(plugin, "diagExport")).toEqual([{ scope: "last" }, { scope: "all" }]);
    fireEvent.click(screen.getByTestId("diag-mark-crackle"));
    await settle();
    expect(calls(plugin, "diagMark")).toEqual([{ label: "crackle" }]);
    // Marks are fixed chips: the section has no text input at all.
    expect(screen.getByTestId("settings-screen").querySelectorAll("[data-testid^=diag] input, textarea").length).toBe(0);
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    fireEvent.click(screen.getByRole("button", { name: "Clear diagnostics" }));
    await settle();
    expect(calls(plugin, "diagClear")).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "Clear diagnostics" }));
    await settle();
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(calls(plugin, "diagClear")).toHaveLength(1);
  });

  it("keeps the Field test visible", () => {
    mountWith(<SettingsScreen params={{}} />);
    expect(screen.getByRole("button", { name: /Field test/ })).toBeTruthy();
  });
});
