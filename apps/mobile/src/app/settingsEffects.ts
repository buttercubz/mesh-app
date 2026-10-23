// Applies persisted settings to the runtime (WP-18): UI language (+ <html lang>), theme,
// and the spoken-prompt templates of a running engine (§7.7: re-sent when the language
// or the spoken-prompts switch changes).
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
  return applyTheme(settings.theme);
}
