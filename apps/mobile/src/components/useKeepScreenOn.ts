// Keeps the screen awake while a component is mounted (WP-18 interface; DESIGN §1.3 F3,
// §10.2.3): the lobby, the PTT ride screen and the car "keep screen on while mounted"
// setting. Calls setKeepScreenOn(true) on mount and false on unmount. The bridges reset
// the flag on every background transition, so it is re-applied on foreground return.
import { useEffect } from "react";

import type { AppEngine, ForegroundSource } from "../engine/client";
import { documentForeground, engine } from "../engine/client";

type KeepScreenOnClient = Pick<AppEngine, "setKeepScreenOn">;

function report(e: unknown): void {
  // Keep-awake is a comfort feature: a refusal (unsupported browser) is only logged.
  console.warn("[screen] setKeepScreenOn failed", e);
}

export function useKeepScreenOn(
  on: boolean,
  client: KeepScreenOnClient = engine,
  foreground: ForegroundSource = documentForeground,
): void {
  useEffect(() => {
    if (!on) return;
    const apply = () => {
      client.setKeepScreenOn(true).catch(report);
    };
    apply();
    const offForeground = foreground.onForeground(apply);
    return () => {
      offForeground();
      client.setKeepScreenOn(false).catch(report);
    };
  }, [on, client, foreground]);
}
