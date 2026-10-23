// Settings → Diagnostics (WP-22 + diagnostics flight recorder, ridelog/1): the
// "Record diagnostics" switch (on by default while field testing; the native default),
// the storage line, Export last ride / Export all through the share sheet, "Mark a
// problem now" with fixed chips (never free text: no text chat, no personal data) and
// Clear with a confirmation. Plain copy, ES + EN through the `settings` catalog.
import { Button } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import type { DiagExportScope, DiagMarkLabel, DiagStatus } from "../../engine/client";
import { DIAG_MARK_LABELS } from "../../engine/client";
import { currentLanguage, useT } from "../../i18n/index";
import { useIntercomEngine } from "../group/engine";
import { SettingRow, SettingsGroup, Switch } from "./controls";

const DAY_MS = 86_400_000;

/** "12.4 MB" in the UI language's number format. */
function formatSize(bytes: number, language: string): string {
  const mb = bytes / 1_000_000;
  const n = new Intl.NumberFormat(language, { maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(mb);
  return `${n} MB`;
}

export function DiagnosticsSection() {
  const { t } = useT("settings");
  const engine = useIntercomEngine();
  const [status, setStatus] = useState<DiagStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** Age of the oldest stored session in whole days, taken when the status arrived. */
  const [ageDays, setAgeDays] = useState(0);

  const refresh = () => {
    const loaded = (s: DiagStatus) => {
      setStatus(s);
      setAgeDays(s.oldestMs === null ? 0 : Math.max(0, Math.floor((Date.now() - s.oldestMs) / DAY_MS)));
    };
    engine.diagStatus().then(loaded, (e: unknown) => {
      // No recorder (platform-only build): the section stays with its defaults.
      console.warn("[settings] diagnostics status unavailable", e);
    });
  };
  useEffect(refresh, [engine]);

  const toggle = (on: boolean) => {
    setStatus((s) => (s === null ? s : { ...s, enabled: on }));
    engine.diagSetEnabled(on).then(refresh, (e: unknown) => {
      console.warn("[settings] diagnostics toggle failed", e);
      refresh();
    });
  };

  const exportScope = (scope: DiagExportScope) => {
    setBusy(true);
    setMessage(null);
    // Called from the tap: the native share sheet opens from this gesture.
    engine.diagExport(scope).then(
      (r) => {
        setBusy(false);
        setMessage(t(r.sessions === 0 && r.path === "" ? "diag.nothing" : "diag.exported"));
      },
      (e: unknown) => {
        console.warn("[settings] diagnostics export failed", e);
        setBusy(false);
        setMessage(t("diag.exportFailed"));
      },
    );
  };

  const mark = (label: DiagMarkLabel) => {
    engine.diagMark(label).then(
      () => setMessage(t("diag.marked")),
      (e: unknown) => console.warn("[settings] diagnostics mark failed", e),
    );
  };

  const clear = () => {
    if (!window.confirm(t("diag.clearConfirm"))) return;
    engine.diagClear().then(
      () => {
        setMessage(t("diag.cleared"));
        refresh();
      },
      (e: unknown) => console.warn("[settings] diagnostics clear failed", e),
    );
  };

  const usage =
    status === null || status.sessions === 0
      ? t("diag.usageEmpty")
      : t("diag.usage", {
          size: formatSize(status.bytes, currentLanguage()),
          sessions: status.sessions,
          days: ageDays,
        });

  return (
    <SettingsGroup title={t("diag.title")}>
      <SettingRow label={t("diag.record")} description={t("diag.recordHint")} testId="diag-record-row">
        <Switch label={t("diag.record")} checked={status?.enabled ?? false} onChange={toggle} disabled={status === null} />
      </SettingRow>
      <div className="flex flex-col gap-2 px-4 py-3 text-sm">
        <p data-testid="diag-usage">{usage}</p>
        <p className="text-muted-foreground">{t("diag.privacy")}</p>
      </div>
      <div className="flex flex-col gap-2 px-4 py-3">
        <Button variant="secondary" block disabled={busy} onClick={() => exportScope("last")}>
          {t("diag.exportLast")}
        </Button>
        <Button variant="secondary" block disabled={busy} onClick={() => exportScope("all")}>
          {t("diag.exportAll")}
        </Button>
      </div>
      <SettingRow label={t("diag.mark")} description={t("diag.markHint")} stacked>
        <div className="flex flex-wrap gap-2" role="group" aria-label={t("diag.mark")}>
          {DIAG_MARK_LABELS.map((label) => (
            <Button key={label} variant="plain" onClick={() => mark(label)} data-testid={`diag-mark-${label}`}>
              {t(`diag.markLabel.${label}`)}
            </Button>
          ))}
        </div>
      </SettingRow>
      <div className="px-4 py-3">
        <Button variant="danger" block onClick={clear}>
          {t("diag.clear")}
        </Button>
      </div>
      {message !== null && (
        <p className="px-4 pb-3 text-sm" role="status">
          {message}
        </p>
      )}
    </SettingsGroup>
  );
}
