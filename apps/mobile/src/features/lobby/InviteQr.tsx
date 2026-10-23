// Invite QR of the lobby (WP-22, DESIGN §1.3 F3, §8.1): the engine renders the QR
// natively (`getInvite` → `qrPng` data URL); the invite text never reaches JS. The image
// lives only in this component's state, hides itself after `hideAfterMs` (60 s) and on
// unmount, and is never stored. A screenshot of the code lets anyone join: the card
// says so.
import { Button } from "@meshapp/ui-kit";
import { useEffect, useState } from "react";

import { useT } from "../../i18n/index";
import { useIntercomEngine } from "../group/engine";
import { showError } from "../group/session";

interface ShownQr {
  readonly png: string;
  readonly hideAfterMs: number;
}

export function InviteQr({ groupId }: { readonly groupId: string }) {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const [qr, setQr] = useState<ShownQr | null>(null);
  const [loading, setLoading] = useState(false);
  const [expired, setExpired] = useState(false);

  // Auto-hide: the QR is membership; it must not stay on screen.
  useEffect(() => {
    if (qr === null) return;
    const timer = setTimeout(() => {
      setQr(null);
      setExpired(true);
    }, qr.hideAfterMs);
    return () => clearTimeout(timer);
  }, [qr]);

  const show = () => {
    setLoading(true);
    engine
      .command({ type: "getInvite", groupId })
      .then(
        (r) => {
          if (r.ok) {
            setQr({ png: r.qrPng, hideAfterMs: r.hideAfterMs });
            setExpired(false);
          } else showError(r.code);
        },
        (e: unknown) => {
          console.warn("[lobby] getInvite failed", e);
          showError(undefined);
        },
      )
      .finally(() => setLoading(false));
  };

  return (
    <section className="rounded-section bg-card p-4 text-card-foreground" aria-labelledby="invite-qr-title" data-testid="invite-qr">
      <div className="flex items-center justify-between gap-3">
        <h2 id="invite-qr-title" className="text-base font-semibold">
          {t("lobby.qrTitle")}
        </h2>
        {qr !== null && (
          <Button variant="plain" onClick={() => setQr(null)}>
            {t("lobby.qrHide")}
          </Button>
        )}
      </div>
      {qr === null ? (
        <div className="mt-3 flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">{expired ? t("lobby.qrHidden") : t("lobby.qrHint")}</p>
          <Button variant="secondary" size="lg" block disabled={loading} onClick={show}>
            {expired ? t("lobby.qrShowAgain") : t("lobby.qrShow")}
          </Button>
        </div>
      ) : (
        <div className="mt-3 flex flex-col items-center gap-3">
          <img
            src={qr.png}
            alt={t("lobby.qrAlt")}
            className="aspect-square w-full max-w-80 rounded-lg bg-white p-3 [image-rendering:pixelated]"
            data-testid="invite-qr-image"
            draggable={false}
          />
          <p role="note" className="text-center text-sm font-medium text-ladder-warn">
            {t("lobby.qrWarning")}
          </p>
          <p className="text-center text-xs text-muted-foreground">
            {t("lobby.qrAutoHide", { s: Math.round(qr.hideAfterMs / 1000) })}
          </p>
        </div>
      )}
    </section>
  );
}
