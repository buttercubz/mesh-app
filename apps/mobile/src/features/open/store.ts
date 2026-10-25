// Open-channel preferences (per device, never sent): the last channel picked and the
// display names this rider blocked. Identities on open channels are ephemeral by design,
// so a block is re-applied by name whenever a rider with that name shows up. No secrets.
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import { appStorage, isRecord } from "../../store/storage";

export const OPEN_CHANNELS = [1, 2, 3, 4, 5, 6, 7, 8, 9] as const;
export type OpenChannel = (typeof OPEN_CHANNELS)[number];
/** Blocked names kept per device. */
export const MAX_BLOCKED_NAMES = 50;

export interface OpenChannelPrefs {
  channel: OpenChannel;
  blockedNames: string[];
}

export function isOpenChannel(n: unknown): n is OpenChannel {
  return typeof n === "number" && Number.isInteger(n) && n >= 1 && n <= 9;
}

/** Rebuilds the preferences from untrusted persisted JSON. */
export function sanitizeOpenPrefs(raw: unknown): OpenChannelPrefs {
  const base: OpenChannelPrefs = { channel: 1, blockedNames: [] };
  if (!isRecord(raw)) return base;
  const names = Array.isArray(raw["blockedNames"]) ? raw["blockedNames"] : [];
  return {
    channel: isOpenChannel(raw["channel"]) ? raw["channel"] : base.channel,
    blockedNames: [...new Set(names.filter((n): n is string => typeof n === "string" && n !== ""))].slice(
      0,
      MAX_BLOCKED_NAMES,
    ),
  };
}

export interface OpenChannelState extends OpenChannelPrefs {
  setChannel(channel: OpenChannel): void;
  setBlocked(name: string, blocked: boolean): void;
}

export const useOpenChannelStore = create<OpenChannelState>()(
  persist(
    (set, get) => ({
      channel: 1,
      blockedNames: [],
      setChannel: (channel) => set({ channel }),
      setBlocked: (name, blocked) => {
        if (name === "") return;
        const rest = get().blockedNames.filter((n) => n !== name);
        set({ blockedNames: blocked ? [name, ...rest].slice(0, MAX_BLOCKED_NAMES) : rest });
      },
    }),
    {
      name: "meshapp.openChannel",
      version: 1,
      storage: createJSONStorage(() => appStorage),
      partialize: (s): OpenChannelPrefs => ({ channel: s.channel, blockedNames: s.blockedNames }),
      merge: (persisted, current) => ({ ...current, ...sanitizeOpenPrefs(persisted) }),
    },
  ),
);
