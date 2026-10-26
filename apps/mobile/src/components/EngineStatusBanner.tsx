// Shell-level engine problems that block everything else: a plugin whose UI API major
// differs from the app's (§3.5: the UI refuses it) and a crashed engine. Rendered inside
// the screen frame (ScreenLayout, under the title bar that owns the top safe area), so it
// takes its height from the body and never pushes the pinned footer off screen. On the
// ride's glance layout it is a bar: the title only, the message one tap away.
import { Banner, Sheet } from "@meshapp/ui-kit";
import { useState } from "react";

import { useT } from "../i18n/index";
import { APP_API_VERSION, selectEngineCrashed, useEngineStore } from "../store/useEngineStore";

export interface EngineStatusBannerProps {
  /** Compact bar (title only; the message opens in a sheet). */
  readonly bar?: boolean;
  /** Classes of the wrapper (only rendered while there is a problem). */
  readonly className?: string;
}

export function EngineStatusBanner({ bar = false, className }: EngineStatusBannerProps) {
  const { t } = useT("common");
  const compat = useEngineStore((s) => s.compat);
  const pluginApi = useEngineStore((s) => s.info?.apiVersion ?? "?");
  const crashed = useEngineStore(selectEngineCrashed);
  const [details, setDetails] = useState(false);
  let title: string;
  let message: string;
  if (compat === "mismatch") {
    title = t("shell.incompatibleTitle");
    message = t("shell.incompatibleMessage", { app: APP_API_VERSION, engine: pluginApi });
  } else if (crashed) {
    title = t("shell.crashedTitle");
    message = t("shell.crashedMessage");
  } else {
    return null;
  }
  return (
    <div className={className} data-testid="engine-status-banner">
      <Banner
        tone="danger"
        title={title}
        message={message}
        {...(bar ? { variant: "bar" as const, onExpand: () => setDetails(true), expanded: details } : {})}
      />
      {bar && (
        <Sheet open={details} onClose={() => setDetails(false)} title={title} closeLabel={t("action.close")}>
          <p className="text-sm text-muted-foreground">{message}</p>
        </Sheet>
      )}
    </div>
  );
}
