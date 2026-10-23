// Lab code entry and the joined state (WP-21, DESIGN §12.1): every phone enters the same
// 4-digit code; Join starts the engine in Lab mode from the tap. Once joined, the card
// shows the code, the linked phones and the Leave action.
import { Button } from "@meshapp/ui-kit";
import { useState } from "react";
import type { FormEvent } from "react";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { LAB_CODE_PATTERN, joinLab, leaveLab } from "./actions";
import { useLabEngine } from "./context";
import { useFieldTestStore } from "./store";

export function LabCodeCard() {
  const { t } = useT("lab");
  const engine = useLabEngine();
  const code = useFieldTestStore((s) => s.code);
  const joining = useFieldTestStore((s) => s.joining);
  const leaving = useFieldTestStore((s) => s.leaving);
  const groupSession = useEngineStore((s) => s.snapshot?.session ?? null);
  const neighbors = useEngineStore((s) => s.snapshot?.mesh.neighbors ?? 0);
  const [draft, setDraft] = useState("");
  const [touched, setTouched] = useState(false);
  const valid = LAB_CODE_PATTERN.test(draft);

  if (code !== null) {
    return (
      <section className="rounded-2xl bg-card p-4" aria-labelledby="lab-code-joined">
        <h2 id="lab-code-joined" className="text-lg font-semibold" data-testid="lab-joined">
          {t("code.joined", { code })}
        </h2>
        <p className="mt-1 text-sm text-muted-foreground" role="status" data-testid="lab-peers">
          {neighbors > 0 ? t("peers.linked", { count: neighbors }) : t("peers.waiting")}
        </p>
        <Button
          className="mt-3"
          variant="secondary"
          block
          disabled={leaving}
          onClick={() => {
            void leaveLab(engine);
          }}
        >
          {leaving ? t("code.leaving") : t("code.leave")}
        </Button>
      </section>
    );
  }

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    setTouched(true);
    if (!valid || groupSession !== null) return;
    // The submit event is the user's tap: joinLab calls startEngine synchronously.
    void joinLab(engine, draft);
  };

  return (
    <form className="rounded-2xl bg-card p-4" onSubmit={onSubmit} noValidate>
      <label htmlFor="lab-code" className="block text-sm font-medium">
        {t("code.label")}
      </label>
      <div className="mt-2 flex gap-2">
        <input
          id="lab-code"
          name="labCode"
          className="min-h-target min-w-0 flex-1 rounded-xl bg-control-fill px-3 text-center font-mono text-2xl tracking-[0.4em]"
          inputMode="numeric"
          autoComplete="off"
          pattern="[0-9]{4}"
          maxLength={4}
          value={draft}
          aria-describedby="lab-code-hint"
          aria-invalid={touched && !valid}
          onChange={(e) => setDraft(e.target.value.replace(/\D/g, "").slice(0, 4))}
        />
        <Button type="submit" variant="primary" disabled={joining || groupSession !== null}>
          {joining ? t("code.joining") : t("code.join")}
        </Button>
      </div>
      <p id="lab-code-hint" className="mt-2 text-xs text-muted-foreground">
        {touched && !valid ? t("code.invalid") : t("code.hint")}
      </p>
      {groupSession !== null && (
        <p role="status" className="mt-2 text-sm text-ladder-warn">
          {t("code.busySession")}
        </p>
      )}
    </form>
  );
}
