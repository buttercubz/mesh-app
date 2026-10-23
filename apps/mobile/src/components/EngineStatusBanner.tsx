// Shell-level engine problems that block everything else: a plugin whose UI API major
// differs from the app's (§3.5: the UI refuses it) and a crashed engine.
import { Banner } from "@meshapp/ui-kit";

import { useT } from "../i18n/index";
import { APP_API_VERSION, selectEngineCrashed, useEngineStore } from "../store/useEngineStore";

export function EngineStatusBanner() {
  const { t } = useT("common");
  const compat = useEngineStore((s) => s.compat);
  const pluginApi = useEngineStore((s) => s.info?.apiVersion ?? "?");
  const crashed = useEngineStore(selectEngineCrashed);
  if (compat === "mismatch") {
    return (
      <Banner
        tone="danger"
        title={t("shell.incompatibleTitle")}
        message={t("shell.incompatibleMessage", { app: APP_API_VERSION, engine: pluginApi })}
      />
    );
  }
  if (crashed) {
    return <Banner tone="danger" title={t("shell.crashedTitle")} message={t("shell.crashedMessage")} />;
  }
  return null;
}
