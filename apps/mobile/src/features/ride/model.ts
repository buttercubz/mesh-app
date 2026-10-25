// Pure derivations from the engine snapshot (WP-22, DESIGN §11.4, §1.3 F3-F6): reach
// counts, ride banners, route labels and the VOX rule. No React, no i18n strings: the
// screens turn the keys returned here into text through the `ride` catalog.
import type {
  AudioRouteKind,
  LadderLevel,
  Member,
  Snapshot,
  TalkDeniedReason,
  TalkMode,
  Vehicle,
} from "@meshapp/engine-api";

/** How long a `talkDenied` keeps the busy banner up after the last refusal (ms). */
export const BUSY_BANNER_MS = 4_000;

/** Every member except this device. */
export function othersOf(snapshot: Snapshot | null): Member[] {
  return (snapshot?.members ?? []).filter((m) => m.status !== "me");
}

/** A member whose path is up (heard recently and confirmed by the leader's roster). */
export function isConnected(m: Member): boolean {
  return m.status === "direct" || m.status === "relayed" || m.status === "oneWay";
}

/** "Heard by k of n": members that report hearing this device. */
export function heardBy(snapshot: Snapshot | null): { k: number; n: number } {
  const others = othersOf(snapshot);
  return { k: others.filter((m) => m.hearsMe && isConnected(m)).length, n: others.length };
}

/**
 * "Voice reaches N of M" (§1.3 F3, §5.5): members whose voice arrives within the budget
 * (`kpi`) or best effort (3-4 hops), with the best-effort share counted apart so the
 * screen can mark it. Lost and unconfirmed members never count.
 */
export function voiceReach(snapshot: Snapshot | null): { n: number; m: number; bestEffort: number } {
  const others = othersOf(snapshot);
  const reached = others.filter((x) => isConnected(x) && x.voice !== "no");
  return {
    n: reached.length,
    m: others.length,
    bestEffort: reached.filter((x) => x.voice === "bestEffort").length,
  };
}

/** The phone's own loudspeaker: VOX is never offered there (§1.4, AEC worst case). */
export function isLoudspeaker(route: AudioRouteKind | null | undefined): boolean {
  return route === "builtinSpeaker";
}

/** Whether VOX may be selected on this route. */
export function voxAllowed(route: AudioRouteKind | null | undefined): boolean {
  return !isLoudspeaker(route);
}

/**
 * Talk mode to apply when the ride starts (§1.4): the rider's default, except VOX on a
 * loudspeaker route, which falls back to PTT.
 */
export function startMode(defaultMode: TalkMode, route: AudioRouteKind | null | undefined): TalkMode {
  return defaultMode === "vox" && !voxAllowed(route) ? "ptt" : defaultMode;
}

/** Whether the ride screen keeps the display awake (§1.3 F4, §1.4). */
export function keepRideScreenOn(mode: TalkMode | undefined, vehicle: Vehicle, keepOnMounted: boolean): boolean {
  return mode === "ptt" || (vehicle === "car" && keepOnMounted);
}

/** i18n key (namespace `ride`) of an audio route. */
export function routeKey(route: AudioRouteKind | null | undefined): string {
  return `route.${route ?? "none"}`;
}

export type BannerKind =
  | "bluetoothOff"
  | "bluetoothDenied"
  | "ladderL1"
  | "ladderL2"
  | "ladderL3"
  | "micPaused"
  | "notLive"
  | "txMutedNoHeadset"
  | "alone"
  | "island"
  | "busy"
  | "preempted"
  | "congestion"
  | "weakSignal";

/**
 * The fix a banner offers. `resumeMic` / `goLive` are ★ gesture commands; `usePhoneMic`
 * is the plain `setBuiltinMicTx {allowed: true}` (capture already runs, only the TX gate
 * opens), so it needs no gesture.
 */
export type BannerAction = "resumeMic" | "goLive" | "openSettings" | "usePhoneMic";

export interface RideBanner {
  readonly kind: BannerKind;
  readonly tone: "info" | "warn" | "blocked" | "danger" | "neutral";
  /** Interpolation params for `banner.<kind>.title` / `.message`. */
  readonly params: Readonly<Record<string, number>>;
  readonly action?: BannerAction;
}

const LADDER: Record<Exclude<LadderLevel, "L0">, RideBanner> = {
  L1: { kind: "ladderL1", tone: "warn", params: {} },
  L2: { kind: "ladderL2", tone: "danger", params: {}, action: "resumeMic" },
  L3: { kind: "ladderL3", tone: "danger", params: {}, action: "resumeMic" },
};

export interface BannerInput {
  readonly snapshot: Snapshot | null;
  /** Local time (ms) of the last `talkDenied`, or `null`. */
  readonly lastTalkDeniedAt: number | null;
  /** Why the last talk request was denied (`cap` when unknown). */
  readonly lastTalkDeniedReason?: TalkDeniedReason | null;
  readonly now: number;
}

/** Path quality below which a link is weak (`Q_GOOD`, three bars, DESIGN §5.1). */
export const Q_GOOD = 60;

/**
 * Every member this device reaches is behind a weak link (edge of range, a fading
 * null): a refusal then comes from the radio, not from too many talkers.
 */
export function weakPath(snapshot: Snapshot | null): boolean {
  const connected = othersOf(snapshot).filter(isConnected);
  return connected.length > 0 && connected.every((m) => m.quality < Q_GOOD);
}

/** Whether the busy banner shows ("Channel full — n talking"). */
export function isBusy(input: BannerInput): boolean {
  const session = input.snapshot?.session;
  if (session?.talk === "blocked") return true;
  return input.lastTalkDeniedAt !== null && input.now - input.lastTalkDeniedAt < BUSY_BANNER_MS;
}

/**
 * The banners of the ride (and lobby) screen, most urgent first: a rider is never mute
 * or deaf without knowing it (§1 principle). Each carries the action that fixes it.
 */
export function rideBanners(input: BannerInput): RideBanner[] {
  const s = input.snapshot;
  if (s === null) return [];
  const out: RideBanner[] = [];
  const session = s.session;
  if (s.device.bluetooth === "off") out.push({ kind: "bluetoothOff", tone: "danger", params: {} });
  if (s.device.bluetooth === "unauthorized") {
    out.push({ kind: "bluetoothDenied", tone: "danger", params: {}, action: "openSettings" });
  }
  // The built-in mic held off by the wind policy (§1.4) is a setting, not a capture
  // failure: its own banner explains it, so "mic paused" is never shown for it.
  const windMuted = session?.live === true && s.audio.txMutedNoHeadset;
  if (session !== null && session.ladder !== "L0") out.push(LADDER[session.ladder]);
  else if (session?.live === true && !s.audio.captureOk && !windMuted) {
    out.push({ kind: "micPaused", tone: "danger", params: {}, action: "resumeMic" });
  }
  if (session !== null && !session.live) out.push({ kind: "notLive", tone: "info", params: {}, action: "goLive" });
  if (windMuted) out.push({ kind: "txMutedNoHeadset", tone: "warn", params: {}, action: "usePhoneMic" });
  const others = othersOf(s).length;
  const alone = others > 0 && othersOf(s).every((m) => !isConnected(m));
  if (alone) out.push({ kind: "alone", tone: "warn", params: {} });
  else if (s.mesh.islands > 1) out.push({ kind: "island", tone: "warn", params: { n: s.mesh.islands } });
  if (isBusy(input)) {
    const reason = input.lastTalkDeniedReason ?? "cap";
    // A full channel means at least the BLE cap (2) is talking somewhere on the path,
    // even when this phone hears fewer of them.
    if (reason === "cap") out.push({ kind: "busy", tone: "blocked", params: { n: Math.max(s.talkers.length, 2) } });
    // A "congestion" refusal over weak links is the radio, not traffic (field test
    // 2026-10-24): say "weak signal", which the rider can fix by closing the gap.
    else if (reason === "congestion" && weakPath(s)) out.push({ kind: "weakSignal", tone: "warn", params: {} });
    else out.push({ kind: reason, tone: "blocked", params: {} });
  }
  return out;
}

/** Compass point (8 sectors) of a bearing in degrees, as an i18n key suffix. */
export function compassKey(bearingDeg: number): string {
  const points = ["n", "ne", "e", "se", "s", "sw", "w", "nw"] as const;
  const i = Math.round((((bearingDeg % 360) + 360) % 360) / 45) % 8;
  return `compass.${points[i] ?? "n"}`;
}

/** Metres, rounded for a glance: 10 m steps under 1 km, then 0.1 km. */
export function formatDistance(m: number, locale: string): string {
  if (m < 1000) return `${Math.max(0, Math.round(m / 10) * 10)} m`;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(m / 1000)} km`;
}

/** Battery 0..1 → whole percent, or `null`. */
export function batteryPercent(battery: number | null | undefined): number | null {
  if (battery === null || battery === undefined || !Number.isFinite(battery)) return null;
  return Math.round(Math.min(1, Math.max(0, battery)) * 100);
}

/** Path quality 0..100 → 0..4 bars (Q_GOOD = 60 is three bars). */
export function qualityBars(quality: number): number {
  if (quality <= 0) return 0;
  if (quality < 30) return 1;
  if (quality < 60) return 2;
  if (quality < 85) return 3;
  return 4;
}

/**
 * Display priority of the banners (lower first). The ride screen shows only the most
 * urgent one in full; the rest collapse into the status chip. Being mute or deaf without
 * knowing it comes first, then refused talk (it answers the rider's own press), then the
 * reach of the group, then a self-healing restart.
 */
const BANNER_PRIORITY: Readonly<Record<BannerKind, number>> = {
  bluetoothOff: 0,
  bluetoothDenied: 1,
  ladderL3: 2,
  ladderL2: 3,
  micPaused: 4,
  notLive: 5,
  txMutedNoHeadset: 6,
  busy: 7,
  preempted: 8,
  congestion: 9,
  weakSignal: 10,
  alone: 11,
  island: 12,
  ladderL1: 13,
};

/** The single banner shown in full and the ones that collapse (both most urgent first). */
export function splitBanners(banners: readonly RideBanner[]): { primary: RideBanner | null; rest: RideBanner[] } {
  const sorted = [...banners].sort((a, b) => BANNER_PRIORITY[a.kind] - BANNER_PRIORITY[b.kind]);
  return { primary: sorted[0] ?? null, rest: sorted.slice(1) };
}
