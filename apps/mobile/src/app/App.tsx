// App root (WP-18, DESIGN §11.3): the single engine event subscription, the screen stack
// and the shell chrome (toasts). The shell engine banner lives inside each screen's frame
// (ScreenLayout), under the title bar, so it never adds height on top of a full-viewport
// frame. No StrictMode: it double-invokes effects that drive the native engine (convLink rule).
import { createElement } from "react";

import { AppToaster } from "../components/AppToaster";
import { useEngineEvents } from "../engine/client";
import { useT } from "../i18n/index";
import { useEngineStore } from "../store/useEngineStore";
import type { Route } from "./router";
import { getScreen, useRouter } from "./router";
import { ShellScreen } from "./ShellScreen";

export function App() {
  useEngineEvents();
  const { t } = useT("common");
  const top = useRouter((s) => s.stack[s.stack.length - 1]);
  // Re-render when a screen registers late (hot reload, lazy features).
  useRouter((s) => s.registryVersion);
  const platform = useEngineStore((s) => s.info?.platform ?? "");
  const engineKind = useEngineStore((s) => s.info?.engine ?? "");
  const compat = useEngineStore((s) => s.compat);
  return (
    <div
      className="h-full bg-background text-foreground"
      data-screen={top?.id ?? ""}
      data-engine-platform={platform}
      data-engine-kind={engineKind}
      data-api-compat={compat}
    >
      <main aria-label={t("a11y.mainContent")} className="h-full">
        {top && <RouteView key={top.key} route={top} />}
      </main>
      <AppToaster />
    </div>
  );
}

/** Renders the registered component of one route (the shell placeholder when none). */
function RouteView({ route }: { readonly route: Route }) {
  const component = getScreen(route.id)?.component ?? ShellScreen;
  return createElement(component, { params: route.params });
}
