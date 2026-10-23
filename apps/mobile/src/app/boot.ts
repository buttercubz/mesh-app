// One-time app bootstrap (WP-18), called by main.tsx before the first render: platform
// class, settings effects, native listeners (back button, deep links) and the plugin's
// getInfo() for the API-version check.
import { setPlatformClass } from "@meshapp/ui-kit";

import { engine } from "../engine/client";
import { installEngineEffects } from "./engineEffects";
import { installNativeHandlers, runtimePlatform } from "./native";
import { installSettingsEffects } from "./settingsEffects";

let booted = false;

export function boot(): void {
  if (booted) return;
  booted = true;
  setPlatformClass(runtimePlatform());
  installSettingsEffects();
  installNativeHandlers();
  installEngineEffects(engine, runtimePlatform());
  engine.getInfo().catch((e: unknown) => {
    // Without getInfo the compat check stays "unknown"; commands still report errors.
    console.error("[engine] getInfo failed", e);
  });
}
