// Procedure text of the Field test (WP-21, DESIGN §12.5): the ES/EN version of
// docs/lab/LAB_PROTOCOL.md (safety, the steps of every run, honesty rules and the
// per-bench notes), from the lab catalog in the UI language.
import { Sheet } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { FIELD_TEST_BENCHES } from "./catalog";

const SAFETY = ["procedure.safety1", "procedure.safety2", "procedure.safety3"] as const;
const STEPS = [1, 2, 3, 4, 5, 6, 7, 8].map((n) => `procedure.step${n}`);
const HONESTY = ["procedure.honesty1", "procedure.honesty2", "procedure.honesty3"] as const;

export function ProcedureContent() {
  const { t } = useT("lab");
  return (
    <div className="flex flex-col gap-4 pb-2 text-sm leading-relaxed" data-testid="lab-procedure">
      <section>
        <h3 className="mb-1 font-semibold">{t("procedure.safetyTitle")}</h3>
        <ul className="list-disc space-y-1 pl-5">
          {SAFETY.map((key) => (
            <li key={key}>{t(key)}</li>
          ))}
        </ul>
      </section>
      <section>
        <h3 className="mb-1 font-semibold">{t("procedure.stepsTitle")}</h3>
        <ol className="list-decimal space-y-1 pl-5">
          {STEPS.map((key) => (
            <li key={key}>{t(key)}</li>
          ))}
        </ol>
      </section>
      <section>
        <h3 className="mb-1 font-semibold">{t("procedure.honestyTitle")}</h3>
        <ul className="list-disc space-y-1 pl-5">
          {HONESTY.map((key) => (
            <li key={key}>{t(key)}</li>
          ))}
        </ul>
      </section>
      <section>
        <h3 className="mb-1 font-semibold">{t("procedure.notesTitle")}</h3>
        <dl className="space-y-2">
          {FIELD_TEST_BENCHES.map((bench) => (
            <div key={bench}>
              <dt className="font-medium">
                {bench} · {t(`bench.${bench}.title`)}
              </dt>
              <dd className="text-muted-foreground">{t(`procedure.note.${bench}`)}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}

export function ProcedureSheet({ onClose }: { readonly onClose: () => void }) {
  const { t } = useT("lab");
  const common = useT("common").t;
  return (
    <Sheet open onClose={() => onClose()} title={t("procedure.title")} closeLabel={common("action.close")}>
      <ProcedureContent />
    </Sheet>
  );
}
