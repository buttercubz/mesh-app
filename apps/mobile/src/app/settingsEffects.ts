// Applies persisted settings to the runtime (WP-18): UI language (+ <html lang>), theme
// (also the native status/navigation bar icons, so they stay readable over the app's
// background), and the spoken-prompt templates of a running engine (§7.7: re-sent when
// the language or the spoken-prompts switch changes).
import { Capacitor, SystemBars, SystemBarsStyle } from "@capacitor/core";
import type { ThemePreference } from "@meshapp/ui-kit";
import { applyTheme } from "@meshapp/ui-kit";

import type { AppEngine } from "../engine/client";
import { engine as appEngine } from "../engine/client";
import { applyLanguage } from "../i18n/index";
import { useEngineStore } from "../store/useEngineStore";
import type { Settings } from "../store/useSettingsStore";
import { useSettingsStore } from "../store/useSettingsStore";

/** Starts applying settings now and on every change; returns the stopper. */
export function installSettingsEffects(engine: Pick<AppEngine, "syncPrompts"> = appEngine): () => void {
  let stopTheme = applyCurrent(useSettingsStore.getState());
  const unsubscribe = useSettingsStore.subscribe((next, prev) => {
    if (next.language !== prev.language) applyLanguage(next.language);
    if (next.theme !== prev.theme) {
      stopTheme();
      stopTheme = applyTheme(next.theme);
      applySystemBars(next.theme);
    }
    const promptsChanged = next.language !== prev.language || next.spokenPrompts !== prev.spokenPrompts;
    if (promptsChanged && useEngineStore.getState().status === "running") {
      engine.syncPrompts().catch((e: unknown) => {
        // The previous templates stay active; earcons are the guaranteed cue anyway.
        console.warn("[settings] setPrompts after a settings change failed", e);
      });
    }
  });
  return () => {
    unsubscribe();
    stopTheme();
  };
}

function applyCurrent(settings: Settings): () => void {
  applyLanguage(settings.language);
  if (typeof document === "undefined") return () => {};
  const stop = applyTheme(settings.theme);
  applySystemBars(settings.theme);
  return stop;
}

/**
 * Light bar icons on the dark theme, dark icons on the light one; "system" follows the
 * OS appearance, as the web theme does. Native only (edge-to-edge bars over the app).
 */
function applySystemBars(theme: ThemePreference): void {
  if (!Capacitor.isNativePlatform()) return;
  const style = theme === "dark" ? SystemBarsStyle.Dark : theme === "light" ? SystemBarsStyle.Light : SystemBarsStyle.Default;
  SystemBars.setStyle({ style }).catch((e: unknown) => {
    // Cosmetic only: the bars keep their previous style (e.g. a build without the plugin).
    console.warn("[settings] SystemBars.setStyle failed", e);
  });
}
