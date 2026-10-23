// Engine used by the intercom screens (WP-22): the app engine by default, a fake-plugin
// engine in component tests (same typed client, same gesture routing, so a ★ command
// still goes through gestureCommand in tests).
import type { SessionInfo } from "@meshapp/engine-api";
import { createContext, useContext } from "react";

import type { AppEngine } from "../../engine/client";
import { engine } from "../../engine/client";
import { useEngineStore } from "../../store/useEngineStore";

export const IntercomEngineContext = createContext<AppEngine>(engine);

export function useIntercomEngine(): AppEngine {
  return useContext(IntercomEngineContext);
}

/**
 * The open session, or `null` while the engine is stopped (a stopped engine leaves its
 * last snapshot in the store; it must not look like a running ride).
 */
export function useSession(): SessionInfo | null {
  return useEngineStore((s) => (s.status === "idle" ? null : (s.snapshot?.session ?? null)));
}
