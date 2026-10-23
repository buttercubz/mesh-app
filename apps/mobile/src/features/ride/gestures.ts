// ★ capture-starting actions of the intercom screens (DESIGN §3.2, AGENTS §8): Go live
// and Resume mic. `engine.goLive()` / `engine.resumeMic()` route through the plugin's
// gestureCommand, and the plugin call happens synchronously inside this function, so it
// must be called straight from a tap handler (never after an await or from a timer).
import { rejectionCode } from "@meshapp/engine-api";

import type { AppEngine } from "../../engine/client";
import { showError } from "../group/session";

export type GestureAction = "goLive" | "resumeMic";

/** Issues the ★ command now (inside the tap) and toasts a refusal. */
export function runGesture(engine: AppEngine, action: GestureAction): Promise<boolean> {
  const pending = action === "goLive" ? engine.goLive() : engine.resumeMic();
  return pending.then(
    (result) => {
      if (result.ok || result.code === "alreadyLive") return true;
      showError(result.code);
      return false;
    },
    (e: unknown) => {
      showError(rejectionCode(e));
      return false;
    },
  );
}
