// Open channel actions (DESIGN v1.x "public channels"): join a numbered public channel
// without a group or QR, and the local (never sent) mute / block of a rider.
import type { Member } from "@meshapp/engine-api";

import { SCREEN, useRouter } from "../../app/router";
import type { AppEngine } from "../../engine/client";
import { useEngineStore } from "../../store/useEngineStore";
import { useSettingsStore } from "../../store/useSettingsStore";
import { runCommand, showError, syncEngineSettings } from "../group/session";
import type { OpenChannel } from "./store";
import { useOpenChannelStore } from "./store";

/**
 * `groupId` of channels 1-9: derived by the core from the public channel context
 * (mesh-crypto `open_channel`, pinned by its `group_ids_are_pinned` test). The UI knows
 * them up front so `startEngine` and `goLive` still leave inside the tap.
 */
export const OPEN_CHANNEL_GROUP_IDS: Readonly<Record<OpenChannel, string>> = {
  1: "d90384ef5ad82a04b0d6f6914135d9ad",
  2: "1e2c165867fd011b98edd952409d8c87",
  3: "c1c13b68546591c798f558aff95c28ab",
  4: "4581af6bd200844ad72d7c900eb2861c",
  5: "e7840e856f7f1fd852a8e14a0d12f883",
  6: "6db408e092582bd92d707f374f19502a",
  7: "09c3f3170a33caad415f2dd34bd2e8b9",
  8: "60f5c0031d57dce432f625b24b5e1b27",
  9: "574449408958d7d684619924cb73bfbe",
};

/**
 * ★ Joins open channel `channel` and goes live (PTT by default) in one tap: the plugin
 * calls `joinOpenChannel`, `startEngine` and `goLive` are all issued synchronously, in
 * that order, before any await. Resolves `true` once the engine runs.
 */
export function joinOpenChannel(engine: AppEngine, channel: OpenChannel): Promise<boolean> {
  const groupId = OPEN_CHANNEL_GROUP_IDS[channel];
  const displayName = useSettingsStore.getState().displayName;
  const prepared = engine.command({ type: "joinOpenChannel", channel, displayName });
  const started = engine.startEngine({ groupId });
  const live = engine.goLive();
  useOpenChannelStore.getState().setChannel(channel);
  return Promise.all([prepared, started]).then(
    async ([r]) => {
      if (!r.ok) {
        showError(r.code);
        return false;
      }
      await syncEngineSettings(engine);
      // The core starts open channels in PTT; keep it whatever the vehicle default is.
      await runCommand(engine, { type: "setTalkMode", mode: "ptt" }, true);
      useRouter.getState().reset(SCREEN.home);
      useRouter.getState().push(SCREEN.ride, { groupId });
      const l = await live.catch(() => null);
      if (l !== null && !l.ok && l.code !== "alreadyLive") console.warn("[open] goLive with the start", l.code);
      return true;
    },
    (e: unknown) => {
      showError(undefined);
      console.warn("[open] joining the channel failed", e);
      return false;
    },
  );
}

/** Local mute (this session only; identities on open channels are ephemeral). */
export function setMuted(engine: AppEngine, m: Member, muted: boolean): Promise<boolean> {
  return runCommand(engine, { type: "setMemberMuted", nodeId: m.nodeId, muted });
}

/** Local block, remembered on this device by display name. */
export function setBlocked(engine: AppEngine, m: Member, blocked: boolean): Promise<boolean> {
  useOpenChannelStore.getState().setBlocked(m.name, blocked);
  return runCommand(engine, { type: "setMemberBlocked", nodeId: m.nodeId, blocked });
}

/**
 * Re-applies the per-device block list to the riders heard now (open channels only).
 * Returns the node ids it blocked.
 */
export function applyBlockedNames(engine: AppEngine): number[] {
  const snap = useEngineStore.getState().snapshot;
  const open = snap?.session?.openChannel ?? null;
  if (snap === null || open === null) return [];
  const names = new Set(useOpenChannelStore.getState().blockedNames);
  const todo = snap.members.filter((m) => m.status !== "me" && names.has(m.name) && !m.flags.includes("blocked"));
  for (const m of todo) void runCommand(engine, { type: "setMemberBlocked", nodeId: m.nodeId, blocked: true }, true);
  return todo.map((m) => m.nodeId);
}
