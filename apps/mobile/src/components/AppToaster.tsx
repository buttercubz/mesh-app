// The app's toast viewport (engine notices and errors) with translated labels. Toasts
// dock at the bottom of the screen so they never cover the title bar or the first content
// row (e.g. the ride screen's "Heard by N of M" line). On screens with a pinned footer
// (ride: PTT/Mute; create/join: confirm) ScreenLayout publishes `--app-toast-bottom` and
// the toasts sit just above the footer; elsewhere they sit above the bottom safe area.
// - The viewport height is capped so a stack can never reach the title bar; only the
//   newest toast (two without a pinned footer) shows, the rest wait behind a "+n" pill.
// - While the ride or lobby is on screen, notices that already have an on-screen banner
//   there (mic lost, Bluetooth off or not allowed, phone mic off for wind) are not toasted
//   twice; the banner is the on-screen copy and the helmet earcon still plays.
import type { ToastItem } from "@meshapp/ui-kit";
import { Toaster } from "@meshapp/ui-kit";
import { useSyncExternalStore } from "react";

import { SCREEN, useRouter } from "../app/router";
import { useT } from "../i18n/index";
import { readToastDock, subscribeToastDock } from "./toastDock";

/** Visible toasts above a pinned footer (the ride's PTT) and elsewhere. */
const MAX_VISIBLE_DOCKED = 1;
const MAX_VISIBLE = 2;

/** Notice toast keys (engine/notices.ts `notice.<slot>`) the ride/lobby banners already show. */
const BANNER_COPIES: ReadonlySet<string> = new Set(["notice.capture", "notice.bluetooth", "notice.bluetoothDenied", "notice.txMutedNoHeadset"]);

/** A banner copy: same slot and the alarming tone (the "back on" success toasts still show). */
function isBannerCopy(item: ToastItem): boolean {
  return item.key !== undefined && BANNER_COPIES.has(item.key) && item.tone === "danger";
}

export function AppToaster() {
  const { t } = useT("common");
  const pinned = useSyncExternalStore(subscribeToastDock, readToastDock, readToastDock);
  const screen = useRouter((s) => s.stack[s.stack.length - 1]?.id ?? "");
  const bannersShown = screen === SCREEN.ride || screen === SCREEN.lobby;
  return (
    <Toaster
      className="top-auto bottom-[var(--app-toast-bottom,max(env(safe-area-inset-bottom),1rem))] max-h-[calc(100dvh-var(--app-toast-bottom,max(env(safe-area-inset-bottom),1rem))-env(safe-area-inset-top)-4.5rem)] justify-end overflow-hidden"
      label={t("a11y.notifications")}
      dismissLabel={t("action.dismiss")}
      maxVisible={pinned ? MAX_VISIBLE_DOCKED : MAX_VISIBLE}
      moreLabel={(n) => t("toast.more", { n })}
      {...(bannersShown ? { filter: (item: ToastItem) => !isBannerCopy(item) } : {})}
    />
  );
}
