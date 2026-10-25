// Member presentation shared by the lobby ready list and the Members screen (WP-22,
// DESIGN §11.4, §1.1 "honest reachability"): i18n keys (namespace `group`) and badge
// tones for status, voice reach and flags. Pure functions; the screens translate.
import type { BadgeTone } from "@meshapp/ui-kit";
import type { Member, MemberFlag, MemberStatus, VoiceReach } from "@meshapp/engine-api";

export interface Label {
  readonly key: string;
  readonly params: Readonly<Record<string, string | number>>;
}

/** Whole seconds since a member was last heard ("last heard 40 s"). */
export function lastHeardSeconds(ms: number): number {
  return Math.max(0, Math.round(ms / 1000));
}

/** Reachability text: direct, relayed n hops, one-way, lost (last heard), unconfirmed. */
export function statusLabel(m: Member): Label {
  switch (m.status) {
    case "me":
      return { key: "member.status.me", params: {} };
    case "direct":
      return { key: "member.status.direct", params: {} };
    case "relayed":
      // C1: `hops` counts link traversals from this device (direct = 1), so a relayed
      // member is at least 2 hops away whatever an older core reports.
      return { key: "member.status.relayed", params: { n: Math.max(m.hops, 2) } };
    case "oneWay":
      return { key: "member.status.oneWay", params: {} };
    case "lost":
      return { key: "member.status.lost", params: { s: lastHeardSeconds(m.lastHeardMs) } };
    case "unconfirmed":
      return { key: "member.status.unconfirmed", params: {} };
  }
}

const STATUS_TONE: Record<MemberStatus, BadgeTone> = {
  me: "brand",
  direct: "active",
  relayed: "relayed",
  oneWay: "warn",
  lost: "out",
  unconfirmed: "warn",
};

export function statusTone(status: MemberStatus): BadgeTone {
  return STATUS_TONE[status];
}

const VOICE_TONE: Record<VoiceReach, BadgeTone> = { kpi: "active", bestEffort: "warn", no: "out" };

export function voiceTone(voice: VoiceReach): BadgeTone {
  return VOICE_TONE[voice];
}

/**
 * Active transport on the path to a reachable member (`member.via.wifi` when a Wi-Fi Aware
 * link carries it, Android↔Android beta; `member.via.ble` otherwise); null for me and for
 * members not reached now.
 */
export function transportKey(m: Member): string | null {
  if (m.status !== "direct" && m.status !== "relayed") return null;
  if (m.linkKinds.length === 0) return null;
  return m.linkKinds.includes("aware") ? "member.via.wifi" : "member.via.ble";
}

/** `member.voice.<reach>`; a lost member's voice never reaches (whatever was last known). */
export function voiceKey(m: Member): string {
  const reach: VoiceReach = m.status === "lost" || m.status === "unconfirmed" ? "no" : m.voice;
  return `member.voice.${reach}`;
}

/** Flags most urgent first (blocked/muted by me, radio off, mic paused, on a call, no headset). */
export function sortedFlags(flags: readonly MemberFlag[]): MemberFlag[] {
  const order: MemberFlag[] = ["blocked", "muted", "radioOff", "micLost", "onCall", "noHeadset"];
  return order.filter((f) => flags.includes(f));
}

const FLAG_TONE: Record<MemberFlag, BadgeTone> = {
  radioOff: "danger",
  micLost: "danger",
  onCall: "warn",
  noHeadset: "warn",
  muted: "neutral",
  blocked: "blocked",
};

export function flagTone(flag: MemberFlag): BadgeTone {
  return FLAG_TONE[flag];
}

/** Members ordered for a list: me first, the leader next, then by reachability and name. */
export function sortMembers(members: readonly Member[]): Member[] {
  const rank: Record<MemberStatus, number> = { me: 0, direct: 1, relayed: 2, oneWay: 3, unconfirmed: 4, lost: 5 };
  return [...members].sort((a, b) => {
    if (a.status === "me" || b.status === "me") return a.status === "me" ? -1 : 1;
    if (a.role !== b.role) return a.role === "leader" ? -1 : 1;
    const r = rank[a.status] - rank[b.status];
    return r !== 0 ? r : a.name.localeCompare(b.name);
  });
}
