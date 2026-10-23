// The app's toast viewport (engine notices and errors) with translated labels. Toasts
// dock at the bottom of the screen so they never cover the title bar or the first content
// row (e.g. the ride screen's "Heard by N of M" line). On screens with a pinned footer
// (ride: PTT/Mute; create/join: confirm) ScreenLayout publishes `--app-toast-bottom` and
// the toasts sit just above the footer; elsewhere they sit above the bottom safe area.
import { Toaster } from "@meshapp/ui-kit";

import { useT } from "../i18n/index";

export function AppToaster() {
  const { t } = useT("common");
  return <Toaster
      className="top-auto bottom-[var(--app-toast-bottom,max(env(safe-area-inset-bottom),1rem))]"
      label={t("a11y.notifications")}
      dismissLabel={t("action.dismiss")}
    />;
}
