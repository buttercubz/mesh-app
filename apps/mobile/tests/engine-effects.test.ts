// Engine effects (WP-18): notification catalog on boot / language change, and the
// one-time Android battery exemption prompt after the first ride (WP-19, WP-22 requests).
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { installEngineEffects } from "../src/app/engineEffects";
import { useEngineStore } from "../src/store/useEngineStore";
import { useSettingsStore } from "../src/store/useSettingsStore";
import { snapshotFixture } from "./fixtures";

function fakeEngine() {
  const calls: { catalogs: { title: string; templates: Record<string, string> }[]; battery: number } = {
    catalogs: [],
    battery: 0,
  };
  return {
    calls,
    setNotificationCatalog: (title: string, templates: Record<string, string>) => {
      calls.catalogs.push({ title, templates });
      return Promise.resolve();
    },
    requestBatteryExemption: () => {
      calls.battery += 1;
      return Promise.resolve({ granted: true });
    },
  };
}

function rideOnce(): void {
  const riding = snapshotFixture();
  useEngineStore.getState().applySnapshot(riding);
  useEngineStore.getState().applySnapshot(snapshotFixture({ session: { ...riding.session!, ride: "lobby" } }));
}

describe("engine effects", () => {
  let stop: () => void = () => {};
  beforeEach(() => {
    useSettingsStore.getState().reset();
    useEngineStore.getState().reset();
  });
  afterEach(() => stop());

  it("sends the localized notification catalog at install and on language change", () => {
    useSettingsStore.getState().update({ language: "en" });
    const engine = fakeEngine();
    stop = installEngineEffects(engine, "ios");
    useSettingsStore.getState().update({ language: "es" });
    expect(engine.calls.catalogs).toHaveLength(2);
    expect(engine.calls.catalogs[0]?.title).toBe("convLink Voice");
    expect(engine.calls.catalogs[0]?.templates["memberJoined"]).toBe("Joined: {{names}}");
    expect(engine.calls.catalogs[1]?.templates["memberJoined"]).toBe("Se unió a la rodada: {{names}}");
    expect(engine.calls.catalogs[1]?.templates["bluetoothOff"]).toBeDefined();
  });

  it("asks for the battery exemption once, after the first ride ends, on Android only", () => {
    const engine = fakeEngine();
    stop = installEngineEffects(engine, "android");
    expect(useSettingsStore.getState().batteryExemptionAsked).toBe(false);
    rideOnce();
    rideOnce();
    expect(engine.calls.battery).toBe(1);
    expect(useSettingsStore.getState().batteryExemptionAsked).toBe(true);
    stop();
    const ios = fakeEngine();
    useSettingsStore.getState().reset();
    stop = installEngineEffects(ios, "ios");
    rideOnce();
    expect(ios.calls.battery).toBe(0);
  });

  it("sanitises the persisted flag", () => {
    useSettingsStore.getState().update({ batteryExemptionAsked: "yes" as unknown as boolean });
    expect(useSettingsStore.getState().batteryExemptionAsked).toBe(false);
  });
});
