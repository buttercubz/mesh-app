// State banners of the lobby and ride screens (WP-22, DESIGN §11.4): Bluetooth off,
// ladder L1-L3 with Resume, mic paused, not live, phone mic muted (wind), alone,
// islands, channel full. Each banner carries the one action that fixes it; Resume mic
// and Go live are ★ commands and are issued synchronously from the tap. "Use phone mic"
// (wind banner) is the same switch as Settings → Talk with the phone mic.
import { Banner } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { runCommand } from "../group/session";
import { runGesture } from "./gestures";
import type { BannerAction, BannerKind, RideBanner } from "./model";
import { BUSY_BANNER_MS, rideBanners } from "./model";

export interface RideBannersProps {
  /** Banner kinds this screen leaves out (the lobby shows Go live as its own button). */
  readonly hide?: readonly BannerKind[];
}

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

export function RideBanners({ hide = [] }: RideBannersProps) {
  const { t } = useT("ride");
  const engine = useIntercomEngine();
  const snapshot = useEngineStore((s) => s.snapshot);
  const lastDeniedAt = useEngineStore((s) => s.lastTalkDenied?.at ?? null);
  const lastDeniedReason = useEngineStore((s) => s.lastTalkDenied?.reason ?? null);
  const now = useBusyClock(lastDeniedAt);
  const banners = rideBanners({
    snapshot,
    lastTalkDeniedAt: lastDeniedAt,
    lastTalkDeniedReason: lastDeniedReason,
    now,
  }).filter(
    (b) => !hide.includes(b.kind),
  );
  if (banners.length === 0) return null;

  const onAction = (action: BannerAction) => {
    if (action === "openSettings") {
      engine.openAppSettings().catch((e: unknown) => {
        // No app settings page (browser): the banner text already says what to do.
        console.warn("[ride] openAppSettings failed", e);
      });
      return;
    }
    if (action === "usePhoneMic") {
      // Not a ★ command: capture already runs, this only opens the TX gate (§1.4).
      useSettingsStore.getState().update({ builtinMicTx: true });
      void runCommand(engine, { type: "setBuiltinMicTx", allowed: true });
      return;
    }
    runGesture(engine, action);
  };

  return (
    <div className="flex flex-col gap-2" data-testid="ride-banners">
      {banners.map((b: RideBanner) => (
        <div key={b.kind} data-banner={b.kind}>
          <Banner
            tone={b.tone}
            title={t(`banner.${b.kind}.title`, b.params)}
            message={t(`banner.${b.kind}.message`, b.params)}
            {...(b.action !== undefined
              ? { action: { label: t(`banner.action.${b.action}`), onClick: () => onAction(b.action as BannerAction) } }
              : {})}
          />
        </div>
      ))}
    </div>
  );
}
