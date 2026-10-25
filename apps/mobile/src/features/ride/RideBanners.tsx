// State banners of the lobby and ride screens (WP-22, DESIGN §11.4): Bluetooth off,
// ladder L1-L3 with Resume, mic paused, not live, phone mic muted (wind), alone,
// islands, channel full. Only the most urgent banner is shown in full (never a stack);
// the others collapse behind a "n more" toggle. Each banner carries the one action that
// fixes it; Resume mic and Go live are ★ commands and are issued synchronously from the
// tap. "Use phone mic" (wind banner) is the same switch as Settings → Talk with the
// phone mic.
import { Banner, cx } from "@meshapp/ui-kit";
import type { LucideIcon } from "lucide-react";
import {
  BluetoothOff,
  ChevronDown,
  Crown,
  Hand,
  Mic,
  MicOff,
  RefreshCw,
  SignalLow,
  TrafficCone,
  Unlink,
  UserX,
  Wind,
} from "lucide-react";
import { useState } from "react";

import { useT } from "../../i18n/index";
import { useSettingsStore } from "../../store/useSettingsStore";
import { useIntercomEngine } from "../group/engine";
import { runCommand } from "../group/session";
import { runGesture } from "./gestures";
import type { BannerAction, BannerKind, RideBanner } from "./model";
import { useRideBanners } from "./useRideBanners";

const ICONS: Readonly<Record<BannerKind, LucideIcon>> = {
  bluetoothOff: BluetoothOff,
  bluetoothDenied: BluetoothOff,
  ladderL1: RefreshCw,
  ladderL2: MicOff,
  ladderL3: MicOff,
  micPaused: MicOff,
  notLive: Mic,
  txMutedNoHeadset: Wind,
  alone: UserX,
  island: Unlink,
  busy: Hand,
  preempted: Crown,
  congestion: TrafficCone,
  weakSignal: SignalLow,
};

/** One banner with its icon and fix. The ★ actions run synchronously inside the tap. */
export function AlertBanner({ banner, compact = false }: { readonly banner: RideBanner; readonly compact?: boolean }) {
  const { t } = useT("ride");
  const engine = useIntercomEngine();
  const Icon = ICONS[banner.kind];

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

  const action = banner.action;
  return (
    <div data-banner={banner.kind}>
      <Banner
        tone={banner.tone}
        icon={<Icon aria-hidden="true" />}
        title={t(`banner.${banner.kind}.title`, banner.params)}
        {...(compact ? {} : { message: t(`banner.${banner.kind}.message`, banner.params) })}
        {...(action !== undefined ? { action: { label: t(`banner.action.${action}`), onClick: () => onAction(action) } } : {})}
      />
    </div>
  );
}

/** The collapsed banners behind a "n more" toggle (compact: title and fix only). */
export function MoreAlerts({ rest }: { readonly rest: readonly RideBanner[] }) {
  const { t } = useT("ride");
  const [open, setOpen] = useState(false);
  if (rest.length === 0) return null;
  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        className="mx-auto inline-flex min-h-target items-center gap-1.5 rounded-capsule px-4 text-sm font-semibold text-muted-foreground active:bg-control-fill"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        data-testid="more-alerts"
      >
        {open ? t("alerts.less") : t("alerts.more", { n: rest.length })}
        <ChevronDown aria-hidden="true" className={cx("size-4 transition-transform duration-ui", open && "rotate-180")} />
      </button>
      {open && rest.map((b) => <AlertBanner key={b.kind} banner={b} compact />)}
    </div>
  );
}

export interface RideBannersProps {
  /** Banner kinds this screen leaves out (the lobby shows Go live as its own button). */
  readonly hide?: readonly BannerKind[];
}

/** The most urgent banner in full, the rest collapsed. */
export function RideBanners({ hide = [] }: RideBannersProps) {
  const { primary, rest } = useRideBanners(hide);
  if (primary === null) return null;
  return (
    <div className="flex flex-col gap-2" data-testid="ride-banners">
      <AlertBanner banner={primary} />
      <MoreAlerts rest={rest} />
    </div>
  );
}
