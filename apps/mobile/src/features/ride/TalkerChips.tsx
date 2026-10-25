// Who is talking, built for a glance at speed (WP-22, DESIGN §11.4): the first active
// talker big (avatar with a live level ring, name, hop count), up to three more as
// compact chips. When nobody talks, a calm "channel clear" state with the riders who are
// on the channel. Colour is always paired with text.
import type { Talker } from "@meshapp/engine-api";
import { Avatar } from "@meshapp/ui-kit";
import { AudioLines } from "lucide-react";

import { useT } from "../../i18n/index";
import { useEngineStore } from "../../store/useEngineStore";
import { isConnected, othersOf } from "./model";

const MAX_TALKERS = 4;
const MAX_FACES = 5;

interface TalkerViewProps {
  readonly talker: Talker;
  readonly me: boolean;
  readonly myName: string;
}

function useTalkerText(talker: Talker, me: boolean) {
  const { t } = useT("ride");
  const level = Math.round(Math.min(1, Math.max(0, talker.level)) * 100);
  const hops = me ? t("talkers.you") : talker.hops <= 1 ? t("talkers.direct") : t("talkers.hops", { n: talker.hops });
  return { level, hops, name: me ? t("talkers.youName") : talker.name, meterLabel: t("talkers.level", { name: talker.name }) };
}

function LevelMeter({ level, label, className }: { readonly level: number; readonly label: string; readonly className: string }) {
  return (
    <span
      className={`${className} overflow-hidden rounded-full bg-control-fill`}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={level}
    >
      <span className="block h-full rounded-full bg-talk-active transition-[width] duration-100" style={{ width: `${level}%` }} />
    </span>
  );
}

/** The main talker: big, centred, readable at arm's length. */
function HeroTalker({ talker, me, myName }: TalkerViewProps) {
  const text = useTalkerText(talker, me);
  return (
    <li
      className="flex animate-rise flex-col motion-reduce:animate-none items-center gap-4 text-center"
      data-testid="talker-chip"
      data-node-id={talker.nodeId}
    >
      <Avatar name={me ? myName : talker.name} size={112} level={talker.level} speaking />
      <span className="flex max-w-full flex-col items-center gap-1">
        <span className="max-w-full truncate text-4xl font-bold tracking-tight">{text.name}</span>
        <span className="inline-flex items-center gap-1.5 text-base font-medium text-talk-active">
          <AudioLines aria-hidden="true" className="size-4" />
          {text.hops}
        </span>
      </span>
      <LevelMeter level={text.level} label={text.meterLabel} className="sr-only" />
    </li>
  );
}

/** A second (third, fourth) talker: compact chip under the hero. */
function SmallTalker({ talker, me, myName }: TalkerViewProps) {
  const text = useTalkerText(talker, me);
  return (
    <li
      className="flex min-h-target min-w-0 items-center gap-3 rounded-capsule bg-talk-active/12 py-1.5 pr-4 pl-1.5 ring-1 ring-talk-active/30"
      data-testid="talker-chip"
      data-node-id={talker.nodeId}
    >
      <Avatar name={me ? myName : talker.name} size={36} speaking />
      <span className="min-w-0">
        <span className="block truncate text-base font-semibold">{text.name}</span>
        <span className="block text-xs text-muted-foreground">{text.hops}</span>
      </span>
      <LevelMeter level={text.level} label={text.meterLabel} className="h-1.5 w-10 shrink-0" />
    </li>
  );
}

/** Nobody talks: the channel is clear, and these riders are on it. */
function Quiet() {
  const { t } = useT("ride");
  const snapshot = useEngineStore((s) => s.snapshot);
  const on = othersOf(snapshot).filter(isConnected);
  const faces = on.slice(0, MAX_FACES);
  return (
    <div className="flex flex-col items-center gap-4 text-center" data-testid="talkers-quiet-state">
      <div className="relative grid size-24 place-items-center" aria-hidden="true">
        <span className="absolute inset-0 rounded-full border border-separator" />
        <span className="absolute inset-3 rounded-full border border-separator" />
        <span className="grid size-12 place-items-center rounded-full bg-brand-soft text-brand">
          <AudioLines className="size-6" />
        </span>
      </div>
      <div className="flex flex-col gap-1">
        <p className="text-2xl font-semibold" data-testid="talkers-quiet">
          {t("talkers.quiet")}
        </p>
        <p className="text-sm text-muted-foreground">{t("talkers.quietHint")}</p>
      </div>
      {faces.length > 0 && (
        <div className="flex flex-col items-center gap-2">
          <span className="text-xs font-semibold tracking-wider text-muted-foreground uppercase">{t("talkers.onChannel")}</span>
          <ul className="flex -space-x-2" aria-label={t("talkers.onChannel")}>
            {faces.map((m) => (
              <li key={m.nodeId} className="rounded-full ring-2 ring-background" title={m.name}>
                <Avatar name={m.name} size={36} />
                <span className="sr-only">{m.name}</span>
              </li>
            ))}
            {on.length > faces.length && (
              <li className="grid size-9 place-items-center rounded-full bg-muted text-xs font-semibold ring-2 ring-background">
                +{on.length - faces.length}
              </li>
            )}
          </ul>
        </div>
      )}
    </div>
  );
}

export function TalkerChips() {
  const { t } = useT("ride");
  const talkers = useEngineStore((s) => s.snapshot?.talkers ?? null);
  const myNodeId = useEngineStore((s) => s.snapshot?.session?.myNodeId ?? null);
  const myName = useEngineStore((s) => s.snapshot?.members.find((m) => m.status === "me")?.name ?? "");
  const list = (talkers ?? []).slice(0, MAX_TALKERS);
  const [hero, ...others] = list;
  return (
    <section aria-label={t("talkers.title")} aria-live="polite" className="flex w-full flex-col items-center justify-center gap-6">
      {hero === undefined ? (
        <Quiet />
      ) : (
        <>
          <ul className="flex w-full justify-center" data-testid="talkers">
            <HeroTalker talker={hero} me={hero.nodeId === myNodeId} myName={myName} />
          </ul>
          {others.length > 0 && (
            <ul className="flex w-full flex-wrap justify-center gap-2">
              {others.map((talker) => (
                <SmallTalker key={talker.nodeId} talker={talker} me={talker.nodeId === myNodeId} myName={myName} />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
