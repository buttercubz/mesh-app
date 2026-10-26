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

/** The other talkers summarised as "+n" (names for screen readers and as a tooltip). */
function MorePill({ names, className }: { readonly names: readonly string[]; readonly className: string }) {
  return (
    <li
      className={`min-h-target min-w-target shrink-0 place-items-center rounded-capsule bg-talk-active/12 px-3 text-sm font-semibold ring-1 ring-talk-active/30 ${className}`}
      data-testid="talkers-more"
      title={names.join(", ")}
    >
      <span aria-hidden="true">+{names.length}</span>
      <span className="sr-only">{names.join(", ")}</span>
    </li>
  );
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

// Sizes follow the talkers area, a size container (RideScreen): the ride is a fixed glance
// layout, so on short phones (or with a banner) the hero shrinks or turns into one row
// instead of being pushed under the footer. Nothing is dropped silently: secondary talkers
// beyond two become a "+n" pill with their names for screen readers. Only a window too
// short for even one row (phone split-screen) leaves the area empty; the helmet audio and
// the status row still carry the ride.

/** The main talker: big, centred, readable at arm's length; one row when the area is short. */
function HeroTalker({ talker, me, myName }: TalkerViewProps) {
  const text = useTalkerText(talker, me);
  return (
    <li
      className="flex w-full min-w-0 animate-rise flex-col items-center gap-[clamp(0.25rem,3cqh,1rem)] text-center motion-reduce:animate-none [@container(max-height:13rem)]:w-auto [@container(max-height:13rem)]:flex-row [@container(max-height:13rem)]:justify-center [@container(max-height:13rem)]:gap-3 [@container(max-height:13rem)]:text-start"
      data-testid="talker-chip"
      data-node-id={talker.nodeId}
    >
      <Avatar name={me ? myName : talker.name} size="clamp(44px, 30cqh, 112px)" level={talker.level} speaking />
      <span className="flex w-full min-w-0 flex-col items-center gap-1 [@container(max-height:13rem)]:w-auto [@container(max-height:13rem)]:items-start">
        <span className="max-w-full truncate text-[clamp(1.375rem,10cqh,2.25rem)] leading-tight font-bold tracking-tight" title={text.name}>
          {text.name}
        </span>
        <span className="inline-flex items-center gap-1.5 text-base font-medium text-talk-active [@container(max-height:5.5rem)]:hidden">
          <AudioLines aria-hidden="true" className="size-4 shrink-0" />
          {text.hops}
        </span>
      </span>
      <LevelMeter level={text.level} label={text.meterLabel} className="sr-only" />
    </li>
  );
}

/** A second (third) talker: compact chip under the hero, one row, names truncate. */
function SmallTalker({ talker, me, myName }: TalkerViewProps) {
  const text = useTalkerText(talker, me);
  return (
    <li
      className="flex min-h-target max-w-[50%] min-w-0 flex-1 items-center gap-2 rounded-capsule bg-talk-active/12 py-1.5 pr-3 pl-1.5 ring-1 ring-talk-active/30"
      data-testid="talker-chip"
      data-node-id={talker.nodeId}
    >
      <Avatar name={me ? myName : talker.name} size="36px" speaking />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-base font-semibold" title={text.name}>
          {text.name}
        </span>
        <span className="block truncate text-xs text-muted-foreground">{text.hops}</span>
      </span>
      <LevelMeter
        level={text.level}
        label={text.meterLabel}
        className="h-1.5 w-8 shrink-0 [@container(max-width:26rem)]:sr-only"
      />
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
    <div className="flex flex-col items-center gap-[clamp(0.5rem,3cqh,1rem)] text-center" data-testid="talkers-quiet-state">
      {/* Decorative: only when the talkers area has room for it. */}
      <div className="relative hidden size-24 place-items-center [@container(min-height:18rem)]:grid" aria-hidden="true">
        <span className="absolute inset-0 rounded-full border border-separator" />
        <span className="absolute inset-3 rounded-full border border-separator" />
        <span className="grid size-12 place-items-center rounded-full bg-brand-soft text-brand">
          <AudioLines className="size-6" />
        </span>
      </div>
      <div className="flex flex-col gap-1">
        <p className="text-[clamp(1.25rem,8cqh,1.5rem)] leading-tight font-semibold text-balance" data-testid="talkers-quiet">
          {t("talkers.quiet")}
        </p>
        <p className="hidden text-sm text-muted-foreground [@container(min-height:11rem)]:block">{t("talkers.quietHint")}</p>
      </div>
      {faces.length > 0 && (
        <div className="flex flex-col items-center gap-2">
          <span className="hidden text-xs font-semibold tracking-wider text-muted-foreground uppercase [@container(min-height:9rem)]:block">
            {t("talkers.onChannel")}
          </span>
          <ul className="flex -space-x-2" aria-label={t("talkers.onChannel")}>
            {faces.map((m) => (
              <li key={m.nodeId} className="rounded-full ring-2 ring-background" title={m.name}>
                <Avatar name={m.name} size="36px" />
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

/** Secondary talkers shown as chips; more than this become a "+n" pill. */
const MAX_CHIPS = 2;

export function TalkerChips() {
  const { t } = useT("ride");
  const talkers = useEngineStore((s) => s.snapshot?.talkers ?? null);
  const myNodeId = useEngineStore((s) => s.snapshot?.session?.myNodeId ?? null);
  const myName = useEngineStore((s) => s.snapshot?.members.find((m) => m.status === "me")?.name ?? "");
  const list = (talkers ?? []).slice(0, MAX_TALKERS);
  const [hero, ...others] = list;
  const chips = others.length > MAX_CHIPS ? others.slice(0, MAX_CHIPS - 1) : others;
  const extra = others.slice(chips.length);
  const nameOf = (talker: Talker) => (talker.nodeId === myNodeId ? t("talkers.youName") : talker.name);
  return (
    <section
      aria-label={t("talkers.title")}
      aria-live="polite"
      className="flex size-full min-h-0 flex-col items-center justify-center gap-[clamp(0.5rem,4cqh,1.5rem)] [@container(max-height:2.75rem)]:hidden"
    >
      {hero === undefined ? (
        <Quiet />
      ) : (
        <>
          <ul className="flex w-full min-w-0 items-center justify-center gap-3" data-testid="talkers">
            <HeroTalker talker={hero} me={hero.nodeId === myNodeId} myName={myName} />
            {/* Too short for the chips row: the other talkers as one "+n" beside the hero. */}
            {others.length > 0 && <MorePill names={others.map(nameOf)} className="hidden [@container(max-height:8.5rem)]:grid" />}
          </ul>
          {others.length > 0 && (
            <ul className="flex w-full min-w-0 flex-nowrap justify-center gap-2 [@container(max-height:8.5rem)]:hidden">
              {chips.map((talker) => (
                <SmallTalker key={talker.nodeId} talker={talker} me={talker.nodeId === myNodeId} myName={myName} />
              ))}
              {extra.length > 0 && <MorePill names={extra.map(nameOf)} className="grid" />}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
