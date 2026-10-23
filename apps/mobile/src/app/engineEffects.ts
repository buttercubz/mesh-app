// Engine side effects driven by settings and snapshots (WP-18):
// - the native notification catalog (title + notices/errors templates of the current
//   language), sent at boot and on every language change (WP-19 request);
// - the one-time Android battery-optimisation exemption prompt after the first ride
//   that ended (DESIGN §1.3 F0 step 2, WP-22 request).
import type { AppEngine } from "../engine/client";
import { engine as appEngine } from "../engine/client";
import type { AppLanguage } from "../i18n/index";
import { catalogTexts, translate } from "../i18n/index";
import { useEngineStore } from "../store/useEngineStore";
import { useSettingsStore } from "../store/useSettingsStore";

export type EngineEffectsEngine = Pick<AppEngine, "setNotificationCatalog" | "requestBatteryExemption">;

/** Title and templates handed to the native notification layer for `language`. */
export function notificationCatalog(language: AppLanguage): { title: string; templates: Record<string, string> } {
  return {
    title: translate("common", "appName", undefined, language),
    templates: { ...catalogTexts("notices", language), ...catalogTexts("errors", language) },
  };
}

function syncNotificationCatalog(engine: EngineEffectsEngine, language: AppLanguage): void {
  const { title, templates } = notificationCatalog(language);
  engine.setNotificationCatalog(title, templates).catch((e: unknown) => {
    // Platforms without native notifications (web) reject; the built-in fallback stays.
    console.warn("[engine] setNotificationCatalog failed", e);
  });
}

/** Installs the effects; returns the stopper. */
export function installEngineEffects(
  engine: EngineEffectsEngine = appEngine,
  platform: "ios" | "android" | "web" = "web",
): () => void {
  syncNotificationCatalog(engine, useSettingsStore.getState().language);
  const offSettings = useSettingsStore.subscribe((next, prev) => {
    if (next.language !== prev.language) syncNotificationCatalog(engine, next.language);
  });
  const offEngine = useEngineStore.subscribe((next, prev) => {
    const wasRiding = prev.snapshot?.session?.ride === "riding";
    const isRiding = next.snapshot?.session?.ride === "riding";
    if (!wasRiding || isRiding || platform !== "android") return;
    const settings = useSettingsStore.getState();
    if (settings.batteryExemptionAsked) return;
    settings.update({ batteryExemptionAsked: true });
    engine.requestBatteryExemption().catch((e: unknown) => {
      // Asked once by design; a failed prompt is not retried (the rider can use Settings).
      console.warn("[engine] requestBatteryExemption failed", e);
    });
  });
  return () => {
    offSettings();
    offEngine();
  };
}
