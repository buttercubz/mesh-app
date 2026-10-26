// Invite QR of the lobby (WP-22, DESIGN §1.3 F3, §8.1): the engine renders the QR
// natively (`getInvite` → `qrPng` data URL); the invite text never reaches JS. The image
// lives only in this component's state, hides itself after `hideAfterMs` (60 s) and on
// unmount, and is never stored. A screenshot of the code lets anyone join: the card
// says so.
import { Button } from "@meshapp/ui-kit";
import { QrCode, ShieldAlert } from "lucide-react";
import { useEffect, useState } from "react";

import { useT } from "../../i18n/index";
import { useIntercomEngine } from "../group/engine";
import { showError } from "../group/session";

/** Scrolls the freshly shown code into the middle of the scrolling body. */
function revealQr(img: HTMLImageElement) {
  const reduce = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  img.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
}

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
    <section
      className="rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface"
      aria-labelledby="invite-qr-title"
      data-testid="invite-qr"
    >
      <div className="flex items-center gap-3">
        <span aria-hidden="true" className="grid size-12 shrink-0 place-items-center rounded-2xl bg-brand-soft text-brand">
          <QrCode className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 id="invite-qr-title" className="text-base font-semibold">
            {t("lobby.qrTitle")}
          </h2>
          {qr === null && <p className="text-sm text-muted-foreground">{expired ? t("lobby.qrHidden") : t("lobby.qrHint")}</p>}
        </div>
      </div>
      {qr === null ? (
        <Button variant="secondary" size="lg" block disabled={loading} onClick={show} className="mt-4">
          {expired ? t("lobby.qrShowAgain") : t("lobby.qrShow")}
        </Button>
      ) : (
        <div className="mt-4 flex animate-rise flex-col items-center gap-3 motion-reduce:animate-none">
          {/* Sized by height as well as width: the whole code (quiet zone included) must fit
              between the title bar and the pinned lobby footer, at any text size, so a
              scanner reads it in one view. */}
          <img
            src={qr.png}
            alt={t("lobby.qrAlt")}
            className="aspect-square w-[min(100%,18rem,38dvh)] rounded-2xl bg-white p-3 shadow-chrome [image-rendering:pixelated]"
            data-testid="invite-qr-image"
            draggable={false}
            onLoad={(e) => revealQr(e.currentTarget)}
          />
          <p role="note" className="flex items-start gap-2 text-sm font-medium">
            <ShieldAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-ladder-warn" />
            {t("lobby.qrWarning")}
          </p>
          <p className="text-center text-xs text-muted-foreground">
            {t("lobby.qrAutoHide", { s: Math.round(qr.hideAfterMs / 1000) })}
          </p>
          <Button variant="plain" block onClick={() => setQr(null)}>
            {t("lobby.qrHide")}
          </Button>
        </div>
      )}
    </section>
  );
}
