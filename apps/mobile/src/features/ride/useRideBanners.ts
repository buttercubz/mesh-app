// The ride/lobby banners as React state (WP-22): re-derived from the snapshot and the
// last talk refusal, with a timer so the busy banner expires on its own.
import { useEffect, useState } from "react";

import { useEngineStore } from "../../store/useEngineStore";
import type { BannerKind, RideBanner } from "./model";
import { BUSY_BANNER_MS, rideBanners, splitBanners } from "./model";

/** Re-renders once the busy banner is due to expire. */
function useBusyClock(lastDeniedAt: number | null): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (lastDeniedAt === null) return;
    const left = lastDeniedAt + BUSY_BANNER_MS - Date.now();
    if (left <= 0) return;
    const timer = setTimeout(() => setNow(Date.now()), left + 50);
    return () => clearTimeout(timer);
  }, [lastDeniedAt]);
  return Math.max(now, lastDeniedAt ?? 0);
}

/** The current banners (most urgent first) minus `hide`, split into the shown one and the rest. */
export function useRideBanners(hide: readonly BannerKind[] = []): { primary: RideBanner | null; rest: RideBanner[] } {
  const snapshot = useEngineStore((s) => s.snapshot);
  const lastDeniedAt = useEngineStore((s) => s.lastTalkDenied?.at ?? null);
  const lastDeniedReason = useEngineStore((s) => s.lastTalkDenied?.reason ?? null);
  const now = useBusyClock(lastDeniedAt);
  const banners = rideBanners({
    snapshot,
    lastTalkDeniedAt: lastDeniedAt,
    lastTalkDeniedReason: lastDeniedReason,
    now,
  }).filter((b) => !hide.includes(b.kind));
  return splitBanners(banners);
}

