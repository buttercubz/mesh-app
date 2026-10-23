// Intercom session actions shared by Home, Create, Join, Lobby and Ride (WP-22, DESIGN
// §1.3 F1-F3, F7, F8). Every function that needs a user gesture issues its plugin call
// synchronously, before any await, so calling it from a tap handler keeps the gesture.
import type { ErrorCode, UiCommand } from "@meshapp/engine-api";
import { rejectionCode } from "@meshapp/engine-api";
import { toaster } from "@meshapp/ui-kit";

import { SCREEN, useRouter } from "../../app/router";
import type { AppEngine } from "../../engine/client";
import { errorToast } from "../../engine/client";
import { useEngineStore } from "../../store/useEngineStore";
import { resetRideDefaults } from "../ride/defaults";
import { useGroupsStore } from "../../store/useGroupsStore";
import { useSettingsStore } from "../../store/useSettingsStore";

/** Shows the translated error of a refused command as a toast. */
export function showError(code: ErrorCode | undefined): void {
  toaster.show(errorToast(code));
}

/**
 * Runs a non-gesture command and toasts a refusal. Resolves `true` on success. Use it
 * for fire-and-forget controls (mode, settings, leader actions).
 */
export async function runCommand(engine: AppEngine, command: UiCommand, quiet = false): Promise<boolean> {
  try {
    const result = await engine.command(command);
    if (result.ok) return true;
    if (!quiet) showError(result.code);
    return false;
  } catch (e) {
    // A transport failure (no plugin, contract break): tell the rider, keep the UI alive.
    console.warn(`[intercom] ${command.type} failed`, e);
    if (!quiet) showError(rejectionCode(e));
    return false;
  }
}

/**
 * Pushes the rider's settings to the engine (display name, vehicle, talk preferences,
 * route, distance sharing). The engine keeps them per device; a refusal of one setting
 * never blocks the others or the ride.
 */
export async function syncEngineSettings(engine: AppEngine): Promise<void> {
  const s = useSettingsStore.getState();
  const commands: UiCommand[] = [
    { type: "setVehicle", vehicle: s.vehicle },
    { type: "setPttStyle", style: s.pttStyle },
    { type: "setVoxSensitivity", level: s.voxSensitivity },
    { type: "setBuiltinMicTx", allowed: s.builtinMicTx },
    { type: "setRoutePreference", pref: s.routePreference },
    { type: "setShareDistance", enabled: s.shareDistance },
  ];
  if (s.displayName !== "") commands.unshift({ type: "setDisplayName", name: s.displayName });
  for (const command of commands) await runCommand(engine, command, true);
}

export interface OpenSessionOptions {
  /** The saved group to open; omitted for the Demo ride. */
  readonly groupId?: string;
  /** Single-device Demo ride with simulated riders (§1.1). */
  readonly demo?: boolean;
  readonly name?: string;
  readonly role?: "leader" | "member";
  /** Also go live in the same tap (Resume ride after process death, §1.3 F7; Demo). */
  readonly goLive?: boolean;
}

/**
 * Opens a group session: `startEngine({groupId})` (gesture), then the rider's settings,
 * then the lobby (or the ride screen when the ride is already running). Resolves `true`
 * once the engine runs. Call it straight from the tap handler.
 */
export function openSession(engine: AppEngine, opts: OpenSessionOptions): Promise<boolean> {
  const groupId = opts.groupId;
  // Synchronous plugin calls first: still inside the tap.
  const started = engine.startEngine(groupId !== undefined ? { groupId } : { demo: opts.demo === true });
  const live = opts.goLive === true ? engine.goLive() : null;
  const groups = useGroupsStore.getState();
  if (groupId !== undefined && opts.name !== undefined) {
    groups.upsert({ id: groupId, name: opts.name, role: opts.role ?? "member" });
  }
  return started.then(
    async () => {
      if (groupId !== undefined) {
        useGroupsStore.getState().touch(groupId);
        useGroupsStore.getState().setActive(groupId);
      }
      await syncEngineSettings(engine);
      const riding = useEngineStore.getState().snapshot?.session?.ride === "riding";
      useRouter.getState().reset(SCREEN.home);
      useRouter.getState().push(riding ? SCREEN.ride : SCREEN.lobby, groupId !== undefined ? { groupId } : {});
      if (live !== null) {
        const r = await live.catch((e: unknown) => ({ ok: false as const, code: rejectionCode(e) }));
        // The screen offers Go live again; a race with the engine start is harmless.
        if (!r.ok && r.code !== "alreadyLive") console.warn("[intercom] goLive with the start", r.code);
      }
      return true;
    },
    (e: unknown) => {
      showError(rejectionCode(e));
      return false;
    },
  );
}

/** Leaves the ride (§1.3 F8): LEAVE is flooded by the engine stop; the group stays saved. */
export async function leaveSession(engine: AppEngine): Promise<void> {
  try {
    await engine.stopEngine();
  } catch (e) {
    // The engine may already be down (crash, background kill): leaving still succeeds.
    console.warn("[intercom] stopEngine failed", e);
  }
  useGroupsStore.getState().setActive(null);
  useEngineStore.getState().clearTalkDenied();
  resetRideDefaults();
  useRouter.getState().reset(SCREEN.home);
}

/** Refreshes the saved groups from the engine (the engine is the source of truth). */
export async function refreshGroups(engine: AppEngine): Promise<void> {
  try {
    const result = await engine.command({ type: "listGroups" });
    if (result.ok) useGroupsStore.getState().syncFromEngine(result.groups);
  } catch (e) {
    // Keep the persisted list: it is only metadata for Home.
    console.warn("[intercom] listGroups failed", e);
  }
}
