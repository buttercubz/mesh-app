// Live view of the running run (WP-21, DESIGN §12.5): big numbers, 60 s sparklines of
// kbps, loss, RTT p95, RSSI and distance, the keep-alive anchor state, Stop (the phone
// that started the run) and Mark with the passenger/stop safety note. Fed by
// labProgress (2 Hz) through useLabStore.
import type { LabBench, LabLiveSample } from "@meshapp/engine-api";
import { Badge, Button, Sparkline } from "@meshapp/ui-kit";
import { useState } from "react";

import { useT } from "../../i18n/index";
import { useLabStore } from "../../store/useLabStore";
import { stopRun } from "./actions";
import { benchSpec } from "./catalog";
import { useLabEngine } from "./context";
import type { LabFormat } from "./format";
import { labFormat } from "./format";
import type { LiveMetric } from "./live";
import { SPARK_METRICS, anchorDropped, series } from "./live";
import { MarkSheet } from "./MarkSheet";
import { useFieldTestStore } from "./store";
import { LAB_THRESHOLDS } from "./thresholds";

type Translate = ReturnType<typeof useT>["t"];

const UNIT: Record<LiveMetric, string> = {
  txKbps: "live.unit.kbps",
  loss: "live.unit.pct",
  rttP95Ms: "live.unit.ms",
  rssi: "live.unit.dbm",
  distM: "live.unit.m",
};

/** Formatted value of a metric (loss as a percentage), or the dash. */
function shown(t: Translate, f: LabFormat, metric: LiveMetric | "rxKbps", v: number | null): string {
  if (v === null) return t("live.none");
  switch (metric) {
    case "txKbps":
    case "rxKbps":
      return f.num(v, 1);
    case "loss":
      return f.pct(v, 1);
    case "rttP95Ms":
      return f.num(v, 0);
    case "rssi":
      return f.num(v, 0);
    case "distM":
      return f.num(v, 0);
  }
}

/** Loss threshold of the bench (dashed line on the loss sparkline). */
function lossThreshold(bench: LabBench | null): number | undefined {
  if (bench === null) return undefined;
  const entry = (LAB_THRESHOLDS as Record<string, unknown>)[bench];
  if (typeof entry !== "object" || entry === null) return undefined;
  const v = (entry as Record<string, unknown>)["maxLoss"];
  return typeof v === "number" ? v : undefined;
}

function BigNumber({ label, value, unit, testId }: { label: string; value: string; unit: string; testId: string }) {
  return (
    <div className="flex flex-col rounded-xl bg-surface-inset p-3" data-testid={testId}>
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-3xl font-semibold tabular-nums">
        {value}
        <span className="ml-1 text-sm font-normal text-muted-foreground">{unit}</span>
      </span>
    </div>
  );
}

export function LiveView() {
  const { t, language } = useT("lab");
  const f = labFormat(language);
  const engine = useLabEngine();
  const samples = useLabStore((s) => s.samples);
  const activeRunId = useLabStore((s) => s.activeRunId);
  const runs = useLabStore((s) => s.runs);
  const started = useFieldTestStore((s) => s.started);
  const stopping = useFieldTestStore((s) => s.stopping);
  const [marking, setMarking] = useState(false);

  const runId = started?.runId ?? activeRunId;
  if (runId === null) return null;
  const own = started !== null && started.runId === runId;
  const bench = own ? started.bench : (runs.find((r) => r.runId === runId)?.bench ?? null);
  const durationS = own ? (started.params.durationS ?? null) : null;
  const points = activeRunId === runId ? samples : [];
  const latest: LabLiveSample | undefined = points[points.length - 1]?.sample;
  const elapsedS = points[points.length - 1]?.elapsedS ?? 0;
  const anchorOk = latest?.anchorOk ?? true;
  const dropped = anchorDropped(points);
  const title = bench === null ? null : benchSpec(bench) === undefined ? bench : t(`bench.${bench}.title`);

  return (
    <section className="flex flex-col gap-3 rounded-2xl bg-card p-4" aria-label={t("section.live")} data-testid="lab-live">
      <header className="flex flex-col gap-1">
        <h2 className="text-lg font-semibold">
          {bench === null ? t("live.follower") : t("live.bench", { bench, title })}
        </h2>
        <p className="text-xs text-muted-foreground break-all">{t("live.runId", { runId })}</p>
        <p className="text-2xl font-semibold tabular-nums" data-testid="lab-elapsed">
          {durationS === null
            ? t("live.elapsedOpen", { elapsed: f.duration(elapsedS) })
            : t("live.elapsed", { elapsed: f.duration(elapsedS), duration: f.duration(durationS) })}
        </p>
      </header>

      {latest === undefined ? (
        <p role="status" className="text-sm text-muted-foreground">
          {t("live.waitingSamples")}
        </p>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-2">
            <BigNumber testId="lab-big-tx" label={t("live.txKbps")} value={shown(t, f, "txKbps", latest.txKbps)} unit={t("live.unit.kbps")} />
            <BigNumber testId="lab-big-rx" label={t("live.rxKbps")} value={shown(t, f, "rxKbps", latest.rxKbps)} unit={t("live.unit.kbps")} />
            <BigNumber testId="lab-big-loss" label={t("live.loss")} value={shown(t, f, "loss", latest.loss)} unit={t("live.unit.pct")} />
            <BigNumber testId="lab-big-rtt" label={t("live.rttP95Ms")} value={shown(t, f, "rttP95Ms", latest.rttP95Ms)} unit={t("live.unit.ms")} />
            <BigNumber testId="lab-big-rssi" label={t("live.rssi")} value={shown(t, f, "rssi", latest.rssi)} unit={t("live.unit.dbm")} />
            <BigNumber testId="lab-big-dist" label={t("live.distM")} value={shown(t, f, "distM", latest.distM)} unit={t("live.unit.m")} />
          </div>
          <p className="text-sm text-muted-foreground tabular-nums">
            {t("live.links")} {f.num(latest.links)} · {t("live.peers")} {f.num(latest.peers)} · {t("live.stalls")}{" "}
            {f.num(latest.stalls)}
          </p>
          <div role="status" data-testid="lab-anchor-state" data-anchor-ok={anchorOk && !dropped}>
            {anchorOk && !dropped ? (
              <Badge tone="active">{t("live.anchorOk")}</Badge>
            ) : (
              <Badge tone="danger">{t("live.anchorLost")}</Badge>
            )}
          </div>
          <div className="flex flex-col gap-2" aria-label={t("live.sparkTitle")}>
            {SPARK_METRICS.map((metric) => {
              const values = series(points, metric);
              const now = latest[metric];
              const label = t(`live.${metric}`);
              return (
                <div key={metric} className="flex items-center gap-3">
                  <span className="w-20 shrink-0 text-xs text-muted-foreground">{label}</span>
                  <Sparkline
                    className="min-w-0 flex-1"
                    width={200}
                    height={32}
                    values={values}
                    min={metric === "loss" ? 0 : undefined}
                    threshold={metric === "loss" ? lossThreshold(bench) : undefined}
                    label={t("live.spark", {
                      metric: label,
                      value: `${shown(t, f, metric, typeof now === "number" ? now : null)} ${t(UNIT[metric])}`,
                    })}
                  />
                </div>
              );
            })}
          </div>
        </>
      )}

      <div className="flex flex-col gap-2">
        <div className="flex gap-2">
          <Button className="flex-1" size="lg" onClick={() => setMarking(true)}>
            {t("mark.button")}
          </Button>
          {own && (
            <Button
              className="flex-1"
              size="lg"
              variant="danger"
              disabled={stopping}
              onClick={() => {
                void stopRun(engine);
              }}
            >
              {stopping ? t("live.stopping") : t("live.stop")}
            </Button>
          )}
        </div>
        <p className="text-xs text-muted-foreground">{t("mark.safety")}</p>
      </div>
      {marking && <MarkSheet onClose={() => setMarking(false)} />}
    </section>
  );
}
