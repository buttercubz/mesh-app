// Groups store (WP-18, DESIGN §11.3): group METADATA only (id, name, role, lastUsed),
// persisted so Home can show the last group and offer Resume ride after process death
// (§1.3 F7). It never holds keys, epoch secrets, invites or member data: `merge` and
// `sanitizeGroup` whitelist the fields, so even a tampered storage entry cannot smuggle
// a secret into the app state.
import type { GroupSummary, MemberRole } from "@meshapp/engine-api";
import { MEMBER_ROLES } from "@meshapp/engine-api";
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import { appStorage, isRecord, oneOf } from "./storage";

export interface GroupMeta {
  /** Group id (32 lowercase hex chars, mesh-api). */
  id: string;
  name: string;
  role: MemberRole;
  /** ms since epoch of the last start or join. */
  lastUsed: number;
}

/** The exact persisted field set; anything else is dropped. */
export const GROUP_META_FIELDS = ["id", "name", "role", "lastUsed"] as const satisfies readonly (keyof GroupMeta)[];

const GROUP_ID = /^[0-9a-f]{32}$/;
const MAX_NAME_LENGTH = 64;

/** A clean GroupMeta from untrusted input, or `null` when it is not one. */
export function sanitizeGroup(raw: unknown): GroupMeta | null {
  if (!isRecord(raw)) return null;
  const id = raw["id"];
  const name = raw["name"];
  const lastUsed = raw["lastUsed"];
  if (typeof id !== "string" || !GROUP_ID.test(id)) return null;
  if (typeof name !== "string") return null;
  return {
    id,
    name: name.slice(0, MAX_NAME_LENGTH),
    role: oneOf(raw["role"], MEMBER_ROLES, "member"),
    lastUsed: typeof lastUsed === "number" && Number.isFinite(lastUsed) && lastUsed >= 0 ? lastUsed : 0,
  };
}

export interface GroupsData {
  groups: GroupMeta[];
  /** Group of the ride in progress (set on start, cleared on a clean stop). */
  activeGroupId: string | null;
}

export function sanitizeGroupsData(raw: unknown): GroupsData {
  if (!isRecord(raw)) return { groups: [], activeGroupId: null };
  const list = Array.isArray(raw["groups"]) ? raw["groups"] : [];
  const byId = new Map<string, GroupMeta>();
  for (const item of list) {
    const g = sanitizeGroup(item);
    if (g !== null && (byId.get(g.id)?.lastUsed ?? -1) < g.lastUsed) byId.set(g.id, g);
  }
  const groups = [...byId.values()].sort((a, b) => b.lastUsed - a.lastUsed);
  const active = raw["activeGroupId"];
  const activeGroupId = typeof active === "string" && byId.has(active) ? active : null;
  return { groups, activeGroupId };
}

export interface GroupsState extends GroupsData {
  /** Inserts or updates one group's metadata. */
  upsert(group: { id: string; name: string; role: MemberRole; lastUsed?: number }): void;
  /** Reconciles with the engine's `listGroups` answer (the engine is the source of truth). */
  syncFromEngine(summaries: readonly GroupSummary[], now?: number): void;
  /** Marks a group as used now. */
  touch(id: string, now?: number): void;
  remove(id: string): void;
  /** The ride in progress (Resume ride after process death); `null` after a clean stop. */
  setActive(id: string | null): void;
  /** Most recently used group first. */
  lastUsed(): GroupMeta | null;
  /** Rewrites storage with the sanitised state (drops anything a tampered entry added). */
  scrub(): void;
}

export const GROUPS_STORE_NAME = "meshapp.groups";
export const GROUPS_STORE_VERSION = 1;

export const useGroupsStore = create<GroupsState>()(
  persist(
    (set, get) => ({
      groups: [],
      activeGroupId: null,
      upsert: (group) => {
        const clean = sanitizeGroup({ ...group, lastUsed: group.lastUsed ?? Date.now() });
        if (clean === null) return;
        const others = get().groups.filter((g) => g.id !== clean.id);
        set(sanitizeGroupsData({ groups: [clean, ...others], activeGroupId: get().activeGroupId }));
      },
      syncFromEngine: (summaries, now = Date.now()) => {
        const known = new Map(get().groups.map((g) => [g.id, g]));
        const groups = summaries
          .filter((s) => !s.closed)
          .map((s) => ({ id: s.groupId, name: s.name, role: s.role, lastUsed: known.get(s.groupId)?.lastUsed ?? now }));
        set(sanitizeGroupsData({ groups, activeGroupId: get().activeGroupId }));
      },
      touch: (id, now = Date.now()) =>
        set(
          sanitizeGroupsData({
            groups: get().groups.map((g) => (g.id === id ? { ...g, lastUsed: now } : g)),
            activeGroupId: get().activeGroupId,
          }),
        ),
      remove: (id) =>
        set(
          sanitizeGroupsData({
            groups: get().groups.filter((g) => g.id !== id),
            activeGroupId: get().activeGroupId === id ? null : get().activeGroupId,
          }),
        ),
      setActive: (id) => set({ activeGroupId: id !== null && get().groups.some((g) => g.id === id) ? id : null }),
      lastUsed: () => get().groups[0] ?? null,
      scrub: () => set(sanitizeGroupsData(get())),
    }),
    {
      name: GROUPS_STORE_NAME,
      version: GROUPS_STORE_VERSION,
      storage: createJSONStorage(() => appStorage),
      partialize: (state) => sanitizeGroupsData(state),
      // v0 had no persisted groups store; any older shape is sanitised field by field.
      migrate: (persisted) => sanitizeGroupsData(persisted),
      merge: (persisted, current) => ({ ...current, ...sanitizeGroupsData(persisted) }),
      // Re-persist right after loading, so stray fields never stay in storage.
      onRehydrateStorage: () => (state) => state?.scrub(),
    },
  ),
);
