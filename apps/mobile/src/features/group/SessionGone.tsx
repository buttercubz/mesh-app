// Shown by the lobby and ride screens when no session is open any more (engine stopped,
// removed from the group, group closed): what happened and the way back Home.
import { Button } from "@meshapp/ui-kit";

import { SCREEN, useRouter } from "../../app/router";
import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";

export function SessionGone() {
  const { t } = useT("group");
  const starting = useEngineStore((s) => s.status === "starting" || s.snapshot === null);
  return (
    <section className="mx-auto flex max-w-md flex-col items-center gap-4 py-16 text-center" data-testid="session-gone">
      {starting ? (
        <p role="status" className="text-lg text-muted-foreground">
          {t("session.starting")}
        </p>
      ) : (
        <>
          <h2 className="text-xl font-semibold">{t("session.goneTitle")}</h2>
          <p className="text-muted-foreground">{t("session.goneMessage")}</p>
          <Button variant="primary" size="lg" onClick={() => useRouter.getState().reset(SCREEN.home)}>
            {t("session.home")}
          </Button>
        </>
      )}
    </section>
  );
}
