// Field test screen (WP-21, DESIGN §12.5; Settings → Field test, visible, plain copy):
// Lab code, live view of the running run, the last finished run, the bench list with
// go/no-go thresholds, results with export/share and the procedure text.
import { Button, ListRow, ListSection } from "@meshapp/ui-kit";
import { useState } from "react";

import type { ScreenProps } from "../../app/router";
import { ErrorMessage } from "../../components/ErrorMessage";
import { ScreenLayout } from "../../components/ScreenLayout";
import { useBackHandler } from "../../components/useBackHandler";
import { useT } from "../../i18n/index";
import { useLabStore } from "../../store/useLabStore";
import { BenchConfigSheet } from "./BenchConfigSheet";
import { BenchList } from "./BenchList";
import type { BenchSpec } from "./catalog";
import { useLabEngine } from "./context";
import { FinishedCard } from "./FinishedCard";
import { LabCodeCard } from "./LabCodeCard";
import { LiveView } from "./LiveView";
import { ProcedureSheet } from "./ProcedureSheet";
import { ResultsList } from "./ResultsList";
import { useFieldTestStore } from "./store";
import { useFieldTestLifecycle } from "./useFieldTestLifecycle";

export function FieldTestScreen(_props: ScreenProps) {
  const { t } = useT("lab");
  const engine = useLabEngine();
  useFieldTestLifecycle(engine);
  const error = useFieldTestStore((s) => s.error);
  const finishedId = useLabStore((s) => s.lastFinished?.runId ?? null);
  const [configFor, setConfigFor] = useState<BenchSpec | null>(null);
  const [procedure, setProcedure] = useState(false);
  const [dismissedId, setDismissedId] = useState<string | null>(null);

  // Android back closes an open sheet before it leaves the screen.
  useBackHandler(() => {
    if (configFor !== null) {
      setConfigFor(null);
      return true;
    }
    if (procedure) {
      setProcedure(false);
      return true;
    }
    return false;
  });

  const openConfig = (spec: BenchSpec) => {
    useFieldTestStore.getState().clearError();
    setConfigFor(spec);
  };

  return (
    <ScreenLayout
      title={t("title")}
      action={
        <Button variant="plain" onClick={() => setProcedure(true)} aria-label={t("procedure.open")}>
          <span aria-hidden="true">?</span>
        </Button>
      }
    >
      <div className="mx-auto flex max-w-xl flex-col gap-4 pb-6" data-testid="field-test">
        <p className="text-sm text-muted-foreground">{t("intro")}</p>
        <LabCodeCard />
        {error !== null && configFor === null && <ErrorMessage code={error.code} />}
        <LiveView />
        {finishedId !== null && finishedId !== dismissedId && (
          <FinishedCard onDismiss={() => setDismissedId(finishedId)} />
        )}
        <BenchList onOpen={openConfig} />
        <ResultsList />
        <ListSection title={t("section.procedure")}>
          <ListRow title={t("procedure.open")} trailing={<span aria-hidden="true">›</span>} onClick={() => setProcedure(true)} />
        </ListSection>
      </div>
      {configFor !== null && (
        <BenchConfigSheet key={configFor.bench} spec={configFor} onClose={() => setConfigFor(null)} />
      )}
      {procedure && <ProcedureSheet onClose={() => setProcedure(false)} />}
    </ScreenLayout>
  );
}
