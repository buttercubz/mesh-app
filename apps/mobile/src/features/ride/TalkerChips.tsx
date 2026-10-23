// Speaking chips of the ride screen (WP-22, DESIGN §11.4): up to four active talkers,
// each with name, hop count and a live playout-level bar. Glanceable: big type, colour
// plus text (never colour alone).
import type { Talker } from "@meshapp/engine-api";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";

const MAX_CHIPS = 4;

function Chip({ talker, me }: { readonly talker: Talker; readonly me: boolean }) {
  const { t } = useT("ride");
  const level = Math.round(Math.min(1, Math.max(0, talker.level)) * 100);
  const hops = me ? t("talkers.you") : talker.hops <= 1 ? t("talkers.direct") : t("talkers.hops", { n: talker.hops });
  return (
    <li
      className="flex min-h-target items-center gap-3 rounded-lg bg-talk-active/15 px-3 py-2"
      data-testid="talker-chip"
      data-node-id={talker.nodeId}
    >
      <span className="size-3 shrink-0 animate-talk-pulse rounded-full bg-talk-active" aria-hidden="true" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-lg font-semibold">{me ? t("talkers.youName") : talker.name}</span>
        <span className="block text-xs text-muted-foreground">{hops}</span>
      </span>
      <span
        className="h-2 w-16 shrink-0 overflow-hidden rounded-full bg-control-fill"
        role="meter"
        aria-label={t("talkers.level", { name: talker.name })}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={level}
      >
        <span className="block h-full rounded-full bg-talk-active" style={{ width: `${level}%` }} />
      </span>
    </li>
  );
}

export function TalkerChips() {
  const { t } = useT("ride");
  const talkers = useEngineStore((s) => s.snapshot?.talkers ?? null);
  const myNodeId = useEngineStore((s) => s.snapshot?.session?.myNodeId ?? null);
  const list = (talkers ?? []).slice(0, MAX_CHIPS);
  return (
    <section aria-label={t("talkers.title")} aria-live="polite" className="min-h-28">
      {list.length === 0 ? (
        <p className="py-6 text-center text-lg text-muted-foreground" data-testid="talkers-quiet">
          {t("talkers.quiet")}
        </p>
      ) : (
        <ul className="grid grid-cols-1 gap-2 sm:grid-cols-2" data-testid="talkers">
          {list.map((talker) => (
            <Chip key={talker.nodeId} talker={talker} me={talker.nodeId === myNodeId} />
          ))}
        </ul>
      )}
    </section>
  );
}
