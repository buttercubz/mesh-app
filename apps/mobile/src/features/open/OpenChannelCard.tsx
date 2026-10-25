// Home entry of open channels: a 1-9 channel picker and one button that goes straight to
// the ride screen (live, PTT), with the public notice in plain sight before joining.
import { Banner, Button } from "@meshapp/ui-kit";
import { Globe } from "lucide-react";

import { useT } from "../../i18n/index";
import { useIntercomEngine } from "../group/engine";
import { joinOpenChannel } from "./actions";
import { OPEN_CHANNELS, useOpenChannelStore } from "./store";

export function OpenChannelCard({ disabled = false }: { readonly disabled?: boolean }) {
  const { t } = useT("group");
  const engine = useIntercomEngine();
  const channel = useOpenChannelStore((s) => s.channel);

  return (
    <section
      className="flex flex-col gap-4 rounded-section border border-separator bg-card p-4 text-card-foreground shadow-surface"
      data-testid="open-channel"
      aria-labelledby="open-channel-title"
    >
      <div className="flex items-center gap-4">
        <span aria-hidden="true" className="grid size-12 shrink-0 place-items-center rounded-full bg-brand-soft text-brand">
          <Globe className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 id="open-channel-title" className="text-base font-semibold">
            {t("open.title")}
          </h2>
          <p className="text-sm text-muted-foreground">{t("open.hint")}</p>
        </div>
      </div>
      <div role="radiogroup" aria-label={t("open.pick")} className="grid grid-cols-9 gap-1.5">
        {OPEN_CHANNELS.map((c) => (
          <button
            key={c}
            type="button"
            role="radio"
            aria-checked={c === channel}
            aria-label={t("open.channel", { n: c })}
            disabled={disabled}
            onClick={() => useOpenChannelStore.getState().setChannel(c)}
            className={
              c === channel
                ? "min-h-target rounded-lg bg-brand text-lg font-bold text-brand-foreground disabled:opacity-50"
                : "min-h-target rounded-lg bg-control-fill text-lg font-semibold disabled:opacity-50"
            }
          >
            {c}
          </button>
        ))}
      </div>
      <Banner tone="warn" title={t("open.public")} message={t("open.publicDetail")} />
      <Button
        variant="primary"
        size="lg"
        block
        disabled={disabled}
        // ★ joinOpenChannel, startEngine and goLive all leave inside this tap.
        onClick={() => void joinOpenChannel(engine, channel)}
      >
        {t("open.join", { n: channel })}
      </Button>
    </section>
  );
}
