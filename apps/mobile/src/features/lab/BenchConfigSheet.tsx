// Per-bench config sheet (WP-21, DESIGN §12.1, §12.5): variants (incl. role swap,
// headset, mixable session), the mandatory audio path and keep-alive anchor (bench
// defaults pre-selected), per-run GPS consent, duration and a note. Location is asked
// in the foreground here, at setup. Start sends labStart from the tap (gestureCommand
// whenever audio ≠ off) and the screen stays awake while the sheet is open, i.e. until
// the run has started.
import { Badge, Button, Sheet } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import { ErrorMessage } from "../../components/ErrorMessage";
import { useKeepScreenOn } from "../../components/useKeepScreenOn";
import { useT } from "../../i18n/index";
import { startRun } from "./actions";
import type { BenchSpec } from "./catalog";
import type { BenchDraft } from "./config";
import { defaultDraft, needsLocation, toLabParams, variantLabel, withAnchor, withAxis } from "./config";
import { AnchorControl, AudioControl, AxisControl, GpsControl } from "./ConfigControls";
import { useLabEngine } from "./context";
import { useFieldTestStore } from "./store";
import { criterionParams } from "./thresholds";

export interface BenchConfigSheetProps {
  readonly spec: BenchSpec;
  readonly onClose: () => void;
}

export function BenchConfigSheet({ spec, onClose }: BenchConfigSheetProps) {
  const { t } = useT("lab");
  const common = useT("common").t;
  const engine = useLabEngine();
  const joined = useFieldTestStore((s) => s.code !== null);
  const starting = useFieldTestStore((s) => s.starting);
  const error = useFieldTestStore((s) => s.error);
  const [draft, setDraft] = useState<BenchDraft>(() => defaultDraft(spec));
  const [locationDenied, setLocationDenied] = useState(false);
  const [micGranted, setMicGranted] = useState<boolean | null>(null);
  // Awake from setup until the run has started (the sheet closes on a confirmed start).
  useKeepScreenOn(true, engine);

  const fullAudio = draft.audio === "full";
  useEffect(() => {
    if (!fullAudio) return;
    let live = true;
    engine.checkPermissions().then(
      (status) => {
        if (live) setMicGranted(status.microphone === "granted");
      },
      () => {
        // Unknown permission state: no advice; the engine reports micPermissionDenied.
        if (live) setMicGranted(null);
      },
    );
    return () => {
      live = false;
    };
  }, [fullAudio, engine]);

  /** Applies `next`; when it newly needs location, asks now and reverts on refusal. */
  const update = (next: BenchDraft) => {
    const before = draft;
    setDraft(next);
    if (!needsLocation(next) || needsLocation(before)) return;
    setLocationDenied(false);
    engine.requestPermissions(["location"]).then(
      (status) => {
        if (status.location === "granted") return;
        setLocationDenied(true);
        setDraft((d) => withAnchor(spec, { ...d, gps: false }, d.anchor === "audioLocation" ? spec.anchor : d.anchor));
      },
      () => {
        // No answer from the permission prompt: treat it as not granted.
        setLocationDenied(true);
        setDraft((d) => withAnchor(spec, { ...d, gps: false }, d.anchor === "audioLocation" ? spec.anchor : d.anchor));
      },
    );
  };

  const check = toLabParams(spec, draft);
  const label = variantLabel(spec, draft);
  const variantAxes = spec.axes.filter((a) => a.key !== "anchor");
  const criterion = t(`bench.${spec.bench}.criterion`, criterionParams(spec.bench));

  const onStart = () => {
    // labStart leaves synchronously inside this tap (gestureCommand when audio ≠ off).
    void startRun(engine, spec, draft).then((outcome) => {
      if (outcome.ok) onClose();
    });
  };

  return (
    <Sheet
      open
      onClose={() => onClose()}
      title={t("config.title", { bench: spec.bench })}
      closeLabel={common("action.close")}
    >
      <div className="flex flex-col gap-4 pb-2" data-testid="lab-config" data-bench={spec.bench}>
        <div className="flex flex-col gap-1">
          <p className="font-semibold">
            {t(`bench.${spec.bench}.title`)}{" "}
            {spec.phase > 0 && <Badge tone="brand">{t("bench.phase", { phase: spec.phase })}</Badge>}
          </p>
          <p className="text-sm text-muted-foreground">{t(`bench.${spec.bench}.setup`)}</p>
          <p className="text-xs text-muted-foreground">
            {spec.devices[0] === spec.devices[1]
              ? t("bench.devicesOne", { min: spec.devices[0] })
              : t("bench.devices", { min: spec.devices[0], max: spec.devices[1] })}
            {spec.repetitions > 1 && ` · ${t("bench.repetitions", { count: spec.repetitions })}`}
          </p>
          <p className="text-xs">
            <span className="font-medium">{t("bench.goNoGo")}: </span>
            {criterion}
          </p>
        </div>

        {variantAxes.length > 0 && (
          <fieldset className="flex flex-col gap-3">
            <legend className="mb-1 text-sm font-semibold">{t("config.variant")}</legend>
            {variantAxes.map((axis) => (
              <AxisControl
                key={axis.key}
                axis={axis}
                value={draft.axes[axis.key] ?? axis.values[0] ?? ""}
                onChange={(v) => update(withAxis(spec, draft, axis.key, v))}
              />
            ))}
          </fieldset>
        )}

        <AudioControl value={draft.audio} onChange={(audio) => update({ ...draft, audio })} />
        {fullAudio && micGranted === false && (
          <div className="flex items-center justify-between gap-2 rounded-xl bg-surface-inset p-3 text-sm">
            <span>{t("config.micAdvice")}</span>
            <Button
              onClick={() => {
                engine.requestPermissions(["microphone"]).then(
                  (status) => setMicGranted(status.microphone === "granted"),
                  () => setMicGranted(false),
                );
              }}
            >
              {t("config.allowMic")}
            </Button>
          </div>
        )}
        <AnchorControl spec={spec} value={draft.anchor} onChange={(anchor) => update(withAnchor(spec, draft, anchor))} />
        <GpsControl checked={draft.gps} onChange={(gps) => update({ ...draft, gps })} />
        {locationDenied && (
          <p role="alert" className="text-sm text-danger">
            {t("config.locationDenied")}
          </p>
        )}

        <label className="flex flex-col gap-1 text-sm font-medium">
          {t("config.duration")}
          <input
            name="durationS"
            type="number"
            inputMode="numeric"
            min={1}
            step={1}
            className="min-h-target rounded-xl bg-control-fill px-3 font-normal"
            value={Number.isFinite(draft.durationS) ? String(draft.durationS) : ""}
            onChange={(e) => setDraft({ ...draft, durationS: e.target.value === "" ? Number.NaN : Number(e.target.value) })}
          />
        </label>
        <label className="flex flex-col gap-1 text-sm font-medium">
          {t("config.note")}
          <input
            name="note"
            type="text"
            maxLength={64}
            autoComplete="off"
            placeholder={t("config.notePlaceholder")}
            className="min-h-target rounded-xl bg-control-fill px-3 font-normal"
            value={draft.note}
            onChange={(e) => setDraft({ ...draft, note: e.target.value })}
          />
        </label>
        {label !== null && (
          <p className="text-xs text-muted-foreground break-all">{t("config.variantLabel", { label })}</p>
        )}

        <p className="text-xs text-muted-foreground">
          <span className="font-medium">{t("bench.captured")}: </span>
          {t(`bench.${spec.bench}.captured`)}
        </p>

        {!check.ok && (
          <p role="alert" className="text-sm text-danger" data-testid="lab-config-problem">
            {t(`config.problem.${check.problem}`)}
          </p>
        )}
        {error !== null && <ErrorMessage code={error.code} />}
        {!joined && (
          <p role="status" className="text-sm text-ladder-warn">
            {t("config.notJoined")}
          </p>
        )}
        <Button variant="primary" size="lg" block disabled={!check.ok || !joined || starting} onClick={onStart}>
          {starting ? t("config.starting") : t("config.start")}
        </Button>
        <p className="text-center text-xs text-muted-foreground">{t("config.keepAwake")}</p>
      </div>
    </Sheet>
  );
}
