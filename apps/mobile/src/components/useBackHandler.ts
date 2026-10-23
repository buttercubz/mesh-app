// Lets a screen or sheet consume the Android back press while mounted (convLink
// useBackButton pattern): return `true` from the handler when it handled the press.
import { useEffect, useRef } from "react";

import type { BackHandler } from "../app/router";
import { pushBackHandler } from "../app/router";

export function useBackHandler(handler: BackHandler, enabled = true): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    if (!enabled) return;
    return pushBackHandler(() => latest.current());
  }, [enabled]);
}
