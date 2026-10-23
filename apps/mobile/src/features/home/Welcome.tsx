// First-launch welcome on Home (WP-22, DESIGN §1.3 F0, §0.2): what the app is, the
// honest range promise, and the way into onboarding (about 90 s, foreground).
import { Button } from "@meshapp/ui-kit";

import { useRouter } from "../../app/router";
import { useT } from "../../i18n/index";
import { ONBOARDING_SCREEN } from "../onboarding/ids";

export function Welcome() {
  const { t } = useT("onboarding");
  const { t: tc } = useT("common");
  return (
    <section className="mx-auto flex max-w-md flex-col gap-6 py-8 text-center" data-testid="welcome">
      <div className="mx-auto grid size-24 place-items-center rounded-full bg-brand text-brand-foreground" aria-hidden="true">
        <svg viewBox="0 0 24 24" className="size-12" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
          <path d="M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3Z" />
          <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
        </svg>
      </div>
      <div className="flex flex-col gap-2">
        <h2 className="text-3xl font-semibold tracking-tight">{t("welcome.title")}</h2>
        <p className="text-lg text-muted-foreground">{tc("shell.tagline")}</p>
      </div>
      <ul className="flex flex-col gap-3 text-start text-base">
        <li>{t("welcome.point1")}</li>
        <li>{t("welcome.point2")}</li>
        <li>{t("welcome.point3")}</li>
      </ul>
      <Button variant="primary" size="lg" block onClick={() => useRouter.getState().push(ONBOARDING_SCREEN)}>
        {t("welcome.start")}
      </Button>
      <p className="text-xs leading-relaxed text-muted-foreground">{t("promise.short")}</p>
    </section>
  );
}
