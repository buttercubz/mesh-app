// Placeholder the shell renders for a screen id no feature has registered (WP-18). In a
// full build WP-22 registers `home`; until then this is the start screen.
import { EngineStatusBanner } from "../components/EngineStatusBanner";
import type { ScreenProps } from "./router";
import { INITIAL_SCREEN, useRouter } from "./router";
import { useT } from "../i18n/index";
import { useEngineStore } from "../store/useEngineStore";

export function ShellScreen(_props: ScreenProps) {
  const { t } = useT("common");
  const info = useEngineStore((s) => s.info);
  const id = useRouter((s) => s.stack[s.stack.length - 1]?.id ?? INITIAL_SCREEN);
  const missing = id !== INITIAL_SCREEN;
  return (
    <section
      data-testid="shell-screen"
      className="mx-auto flex h-dvh max-w-md flex-col justify-center gap-4 overflow-y-auto px-6 pt-[max(env(safe-area-inset-top),2.5rem)] pb-[max(env(safe-area-inset-bottom),2.5rem)] text-center"
    >
      <EngineStatusBanner className="text-start" />
      <h1 className="text-3xl font-semibold tracking-tight">{t("appName")}</h1>
      <p className="text-muted-foreground">{t("shell.tagline")}</p>
      {missing && <p role="status">{t("shell.screenMissing")}</p>}
      <p className="text-sm text-muted-foreground" data-testid="engine-info">
        {info === null
          ? t("shell.engineUnknown")
          : t("shell.engineInfo", {
              engine: info.engine === "full" ? t("shell.engineFull") : t("shell.enginePlatformOnly"),
              platform: info.platform,
              apiVersion: info.apiVersion,
            })}
      </p>
      <p className="text-xs leading-relaxed text-muted-foreground">{t("shell.promise")}</p>
    </section>
  );
}
