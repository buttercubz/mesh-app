// Crafted engine snapshots for the layout audit. They start from a real Snapshot taken
// from the simulator (shape of @meshapp/engine-api, contract C1) and change only the
// fields that drive a screen state: banners, talkers, members, modes.
import type {
  AudioRouteKind,
  BluetoothState,
  EngineState,
  LadderLevel,
  Member,
  MemberFlag,
  MemberRole,
  MemberStatus,
  RidePhase,
  Snapshot,
  TalkMode,
  TalkState,
  Talker,
  VoiceReach,
} from "@meshapp/engine-api";

/** Long but valid display names (≤ 24 UTF-8 bytes), Spanish-style, one very long single word. */
export const RIDER_NAMES = [
  "Alejandro Fernández",
  "Maximiliano Gutiérrez",
  "Guadalupe Ríos",
  "Wolfeschlegelsteinhau",
  "Juan Pablo Hernández",
  "Bartolomé Echeverría",
  "Xóchitl Montenegro",
  "Inés",
  "Constanza Villalobos",
  "José María Peña",
  "Leonardo Santibáñez",
] as const;

export interface OtherSpec {
  readonly name: string;
  readonly status?: MemberStatus;
  readonly hops?: number;
  readonly quality?: number;
  readonly voice?: VoiceReach;
  readonly flags?: MemberFlag[];
  readonly role?: MemberRole;
  readonly hearsMe?: boolean;
  readonly live?: boolean;
  readonly distanceM?: number | null;
  readonly battery?: number | null;
}

export interface Craft {
  readonly engine?: EngineState;
  readonly ride?: RidePhase;
  readonly live?: boolean;
  readonly role?: MemberRole;
  readonly mode?: TalkMode;
  readonly talk?: TalkState;
  readonly ladder?: LadderLevel;
  readonly openChannel?: number | null;
  readonly shareDistance?: boolean;
  readonly groupName?: string;
  readonly route?: AudioRouteKind;
  readonly captureOk?: boolean;
  readonly txMutedNoHeadset?: boolean;
  readonly inputLevel?: number;
  readonly bluetooth?: BluetoothState;
  readonly islands?: number;
  readonly others?: readonly OtherSpec[];
  /** Talker names; "me" is this phone. */
  readonly talkers?: readonly string[];
}

/** The default convoy around this phone: four riders, all heard, one relayed. */
export const DEFAULT_OTHERS: readonly OtherSpec[] = [
  { name: RIDER_NAMES[0], status: "direct", hops: 1, quality: 90, role: "member" },
  { name: RIDER_NAMES[1], status: "direct", hops: 1, quality: 75 },
  { name: RIDER_NAMES[2], status: "relayed", hops: 2, quality: 70, voice: "kpi" },
  { name: RIDER_NAMES[4], status: "relayed", hops: 3, quality: 62, voice: "bestEffort" },
];

/** Twelve riders with every status and flag (members list, lobby ready list). */
export const MANY_OTHERS: readonly OtherSpec[] = [
  { name: RIDER_NAMES[0], status: "direct", hops: 1, quality: 92, distanceM: 120, battery: 0.82 },
  { name: RIDER_NAMES[1], status: "direct", hops: 1, quality: 55, flags: ["onCall"], distanceM: 1850, battery: 0.14 },
  { name: RIDER_NAMES[2], status: "relayed", hops: 2, quality: 70, voice: "kpi", flags: ["noHeadset"] },
  { name: RIDER_NAMES[3], status: "relayed", hops: 4, quality: 35, voice: "bestEffort", flags: ["micLost", "radioOff"] },
  { name: RIDER_NAMES[4], status: "oneWay", hops: 1, quality: 40, hearsMe: false, flags: ["muted"] },
  { name: RIDER_NAMES[5], status: "lost", hops: 0, quality: 0, voice: "no", hearsMe: false, live: false },
  { name: RIDER_NAMES[6], status: "unconfirmed", hops: 1, quality: 80, live: false },
  { name: RIDER_NAMES[7], status: "direct", hops: 1, quality: 88, flags: ["blocked"] },
  { name: RIDER_NAMES[8], status: "relayed", hops: 3, quality: 61, voice: "bestEffort" },
  { name: RIDER_NAMES[9], status: "direct", hops: 1, quality: 99, distanceM: 15 },
  { name: RIDER_NAMES[10], status: "relayed", hops: 2, quality: 66, flags: ["onCall", "noHeadset", "muted"] },
];

function memberOf(spec: OtherSpec, nodeId: number, leaderNode: boolean): Member {
  const status = spec.status ?? "direct";
  const connected = status === "direct" || status === "relayed" || status === "oneWay";
  const live = spec.live ?? true;
  return {
    nodeId,
    name: spec.name,
    role: spec.role ?? (leaderNode ? "leader" : "member"),
    platform: nodeId % 2 === 0 ? "ios" : "android",
    status,
    hops: spec.hops ?? (status === "relayed" ? 2 : 1),
    quality: spec.quality ?? 80,
    linkKinds: ["ble"],
    hearsMe: spec.hearsMe ?? connected,
    voice: spec.voice ?? (connected ? "kpi" : "no"),
    lastHeardMs: connected ? 400 : 45_000,
    battery: spec.battery ?? 0.6,
    ready: { live, mic: live, route: live ? "hfpWideband" : null },
    flags: spec.flags ?? [],
    distanceM: spec.distanceM ?? null,
    bearingDeg: spec.distanceM === undefined || spec.distanceM === null ? null : 135,
  };
}

export function craft(base: Snapshot, c: Craft): Snapshot {
  const s: Snapshot = JSON.parse(JSON.stringify(base)) as Snapshot;
  const baseSession = s.session;
  if (baseSession === null) throw new Error("base snapshot has no session");
  const role = c.role ?? baseSession.role;
  const me = s.members.find((m) => m.status === "me") ?? s.members[0];
  if (me === undefined) throw new Error("base snapshot has no members");
  const myNodeId = baseSession.myNodeId;
  const live = c.live ?? true;
  s.engine = c.engine ?? "running";
  s.session = {
    ...baseSession,
    role,
    live,
    mode: c.mode ?? "ptt",
    talk: c.talk ?? "idle",
    ride: c.ride ?? "riding",
    ladder: c.ladder ?? "L0",
    openChannel: c.openChannel ?? null,
    shareDistance: c.shareDistance ?? false,
    groupName: c.groupName ?? baseSession.groupName,
  };
  s.audio = {
    ...s.audio,
    route: c.route ?? "hfpWideband",
    captureOk: c.captureOk ?? live,
    renderOk: true,
    inputLevel: c.inputLevel ?? 0.35,
    txMutedNoHeadset: c.txMutedNoHeadset ?? false,
  };
  s.device = { ...s.device, bluetooth: c.bluetooth ?? "on", background: false };
  const others = (c.others ?? DEFAULT_OTHERS).map((o, i) => memberOf(o, myNodeId + 101 + i, role !== "leader" && i === 0));
  const meMember: Member = {
    ...me,
    role,
    status: "me",
    ready: { live, mic: live, route: live ? (c.route ?? "hfpWideband") : null },
    flags: [],
  };
  s.members = [meMember, ...others];
  const reachable = others.filter((m) => m.status === "direct" || m.status === "relayed" || m.status === "oneWay").length;
  s.mesh = { ...s.mesh, islands: c.islands ?? 1, reachable, total: others.length + 1, neighbors: Math.min(3, reachable), links: Math.min(3, reachable) };
  s.talkers = (c.talkers ?? []).map((name, i): Talker => {
    if (name === "me") return { nodeId: myNodeId, name: meMember.name, hops: 0, level: 0.6, sinceMs: 1200 };
    const m = others.find((o) => o.name === name);
    return { nodeId: m?.nodeId ?? myNodeId + 900 + i, name, hops: m?.hops ?? 1, level: 0.4 + i * 0.1, sinceMs: 800 * (i + 1) };
  });
  return s;
}
