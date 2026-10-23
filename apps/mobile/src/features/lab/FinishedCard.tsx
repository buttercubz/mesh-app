// Summary of the run that just finished (WP-21, DESIGN §12.3, §12.5): on-device verdict
// chip, the headline numbers and Export (the share sheet), straight from labFinished.
import { Button, toaster } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { useLabStore } from "../../store/useLabStore";
import { exportRun } from "./actions";
import { useLabEngine } from "./context";
import { labFormat } from "./format";
import { VerdictChip } from "./ResultsList";
import { useFieldTestStore } from "./store";

export function FinishedCard({ onDismiss }: { readonly onDismiss: () => void }) {
  const { t, language } = useT("lab");
  const common = useT("common").t;
  const f = labFormat(language);
  const engine = useLabEngine();
  const finished = useLabStore((s) => s.lastFinished);
  const bench = useLabStore((s) => s.runs.find((r) => r.runId === s.lastFinished?.runId)?.bench ?? null);
  const exportingIds = useFieldTestStore((s) => s.exporting);
  if (finished === null) return null;
  const exporting = exportingIds.includes(finished.runId);
  const { summary, files, runId } = finished;
  const none = t("live.none");
  const rows: [string, string][] = [
    [t("finished.duration"), f.duration(summary.durationS)],
    [t("finished.tx"), `${f.num(summary.txKbps.mean, 1)} ${t("live.unit.kbps")}`],
    [t("finished.loss"), `${f.pct(summary.loss, 1)} ${t("live.unit.pct")}`],
    [t("finished.owdP95"), summary.owdMs === null ? none : `${f.num(summary.owdMs.p95, 1)} ${t("live.unit.ms")}`],
    [t("finished.rttP95"), summary.rttMs === null ? none : `${f.num(summary.rttMs.p95, 1)} ${t("live.unit.ms")}`],
    [t("finished.stalls"), f.num(summary.stalls)],
  ];

  const onExport = () => {
    if (files.length === 0) {
      toaster.show({ title: t("results.noFiles"), tone: "warn", key: "lab.export" });
      return;
    }
    void exportRun(engine, runId, files).then((outcome) => {
      if (outcome.ok) toaster.show({ title: t("results.exported"), tone: "success", key: "lab.export" });
    });
  };

  return (
    <section
      className="flex flex-col gap-3 rounded-2xl bg-card p-4"
      aria-labelledby="lab-finished-title"
      data-testid="lab-finished"
      data-run-id={runId}
    >
      <header className="flex items-center justify-between gap-2">
        <h2 id="lab-finished-title" className="text-lg font-semibold">
          {t("finished.title")}
          {bench !== null && <span className="text-muted-foreground"> · {bench}</span>}
        </h2>
        <VerdictChip verdict={summary.verdict} />
      </header>
      <p className="text-xs text-muted-foreground break-all">{t("live.runId", { runId })}</p>
      <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="text-right tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
      <p className="text-xs text-muted-foreground">{t("finished.verdictNote")}</p>
      <div className="flex gap-2">
        <Button className="flex-1" variant="primary" disabled={exporting} onClick={onExport}>
          {exporting ? t("results.exporting") : t("finished.export")}
        </Button>
        <Button variant="plain" onClick={onDismiss}>
          {common("action.dismiss")}
        </Button>
      </div>
    </section>
  );
}
