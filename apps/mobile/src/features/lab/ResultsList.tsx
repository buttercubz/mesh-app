// Results list (WP-21, DESIGN §12.3, §12.5): stored runs, newest first, with verdict
// chips, export through the share sheet and delete. A run without a summary
// (interrupted) is kept and labelled as such (honesty rules, LAB_PROTOCOL).
import type { LabRunInfo, LabVerdict } from "@meshapp/engine-api";
import type { BadgeTone } from "@meshapp/ui-kit";
import { Badge, Button, ListRow, ListSection, toaster } from "@meshapp/ui-kit";

import { useT } from "../../i18n/index";
import { useLabStore } from "../../store/useLabStore";
import { deleteRun, exportRun } from "./actions";
import { useLabEngine } from "./context";
import { labFormat } from "./format";
import { useFieldTestStore } from "./store";

const VERDICT_TONE: Record<LabVerdict | "interrupted", BadgeTone> = {
  go: "active",
  noGo: "danger",
  inconclusive: "warn",
  interrupted: "out",
};

export function VerdictChip({ verdict }: { readonly verdict: LabVerdict | null }) {
  const { t } = useT("lab");
  const key = verdict ?? "interrupted";
  return (
    <Badge tone={VERDICT_TONE[key]} data-verdict={key}>
      {t(`verdict.${key}`)}
    </Badge>
  );
}

function RunRow({ run }: { readonly run: LabRunInfo }) {
  const { t, language } = useT("lab");
  const f = labFormat(language);
  const engine = useLabEngine();
  const exporting = useFieldTestStore((s) => s.exporting.includes(run.runId));
  const active = useLabStore((s) => s.activeRunId === run.runId);
  const startedHere = useFieldTestStore((s) => s.started?.runId === run.runId);
  const running = active || startedHere;

  const onExport = () => {
    if (run.files.length === 0) {
      toaster.show({ title: t("results.noFiles"), tone: "warn", key: "lab.export" });
      return;
    }
    void exportRun(engine, run.runId, run.files).then((outcome) => {
      if (outcome.ok) toaster.show({ title: t("results.exported"), tone: "success", key: "lab.export" });
    });
  };

  const onDelete = () => {
    if (!window.confirm(t("results.confirmDelete", { runId: run.runId }))) return;
    void deleteRun(engine, run.runId);
  };

  return (
    <li className="flex flex-col gap-2 border-b border-separator py-2 last:border-b-0" data-run-id={run.runId}>
      <ListRow
        title={`${run.bench} · ${t(`bench.${run.bench}.title`)}`}
        subtitle={t("results.subtitle", {
          date: f.dateTime(run.startedAt),
          duration: run.durationS === null ? (running ? t("results.running") : t("live.none")) : f.duration(run.durationS),
          audio: t(`audio.${run.audio}`),
          anchor: t(`anchor.${run.anchor}`),
        })}
        trailing={running ? <Badge tone="brand">{t("results.running")}</Badge> : <VerdictChip verdict={run.verdict} />}
      />
      {!running && (
        <div className="flex gap-2 px-3">
          <Button
            className="flex-1"
            disabled={exporting}
            aria-label={t("results.exportRun", { runId: run.runId })}
            onClick={onExport}
          >
            {exporting ? t("results.exporting") : t("results.export")}
          </Button>
          <Button variant="plain" aria-label={t("results.deleteRun", { runId: run.runId })} onClick={onDelete}>
            {t("results.delete")}
          </Button>
        </div>
      )}
    </li>
  );
}

export function ResultsList() {
  const { t } = useT("lab");
  const runs = useLabStore((s) => s.runs);
  return (
    <ListSection title={t("section.results")}>
      {runs.length === 0 ? (
        <p className="p-4 text-sm text-muted-foreground" data-testid="lab-results-empty">
          {t("results.empty")}
        </p>
      ) : (
        <ul data-testid="lab-results" aria-label={t("section.results")}>
          {runs.map((run) => (
            <RunRow key={run.runId} run={run} />
          ))}
        </ul>
      )}
    </ListSection>
  );
}
