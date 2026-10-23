// Mark sheet (WP-21, DESIGN §12.1, §12.5): labels a moment of the running run. The
// passenger/stop safety note is part of the sheet and of the button next to it. Preset
// labels are recorded as stable English tokens (the analysis reads them); the sheet
// shows them translated. A custom label is user content (≤ 64 bytes).
import { Button, Sheet, toaster } from "@meshapp/ui-kit";
import { useState } from "react";
import type { FormEvent } from "react";

import { useT } from "../../i18n/index";
import { MARK_MAX_BYTES, markRun } from "./actions";
import { useLabEngine } from "./context";

/** Preset mark labels, recorded as-is in the labrun/1 `mark` line. */
export const MARK_PRESETS = ["pocket", "mounted", "tankBag", "helmetOff", "helmetOn", "tunnel", "stop", "glitch"] as const;

export function MarkSheet({ onClose }: { readonly onClose: () => void }) {
  const { t } = useT("lab");
  const common = useT("common").t;
  const engine = useLabEngine();
  const [custom, setCustom] = useState("");
  const [busy, setBusy] = useState(false);

  const mark = (label: string, shown: string) => {
    setBusy(true);
    void markRun(engine, label).then((outcome) => {
      setBusy(false);
      if (!outcome.ok) return;
      toaster.show({ title: t("mark.done", { label: shown }), tone: "success", key: "lab.mark" });
      onClose();
    });
  };

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const text = custom.trim();
    if (text !== "") mark(text, text);
  };

  const tooLong = new TextEncoder().encode(custom.trim()).length > MARK_MAX_BYTES;

  return (
    <Sheet open onClose={() => onClose()} title={t("mark.title")} closeLabel={common("action.close")}>
      <div className="flex flex-col gap-3 pb-2" data-testid="lab-mark-sheet">
        <p role="note" className="rounded-xl bg-surface-inset p-3 text-sm">
          {t("mark.safety")}
        </p>
        <div className="grid grid-cols-2 gap-2">
          {MARK_PRESETS.map((preset) => (
            <Button key={preset} disabled={busy} onClick={() => mark(preset, t(`mark.preset.${preset}`))}>
              {t(`mark.preset.${preset}`)}
            </Button>
          ))}
        </div>
        <form className="flex gap-2" onSubmit={onSubmit}>
          <label className="sr-only" htmlFor="lab-mark-custom">
            {t("mark.custom")}
          </label>
          <input
            id="lab-mark-custom"
            className="min-h-target min-w-0 flex-1 rounded-xl bg-control-fill px-3"
            placeholder={t("mark.custom")}
            autoComplete="off"
            maxLength={64}
            value={custom}
            aria-invalid={tooLong}
            onChange={(e) => setCustom(e.target.value)}
          />
          <Button type="submit" variant="primary" disabled={busy || custom.trim() === "" || tooLong}>
            {t("mark.add")}
          </Button>
        </form>
      </div>
    </Sheet>
  );
}
