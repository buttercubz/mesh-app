// Engine store (WP-18, DESIGN §11.3): the last Snapshot plus engine status. Not
// persisted: snapshots are the source of truth and a full one is requested on every
// foreground return. Written only by engine/useEngineEvents.ts and engine/client.ts.
import type {
  AudioCheckPhase,
  AudioRouteKind,
  ErrorCode,
  GateState,
  Snapshot,
  TalkDeniedReason,
} from "@meshapp/engine-api";
import { API_VERSION, API_VERSION_MAJOR } from "@meshapp/engine-api";
import type { IntercomInfo } from "@meshapp/capacitor-intercom";
import { create } from "zustand";

/** Result of comparing the plugin's UI API version with the app's (§3.5, §11.1). */
export type ApiCompat = "unknown" | "ok" | "mismatch";

export interface TalkDenial {
  reason: TalkDeniedReason;
  /** Local receive time (ms), for banner timeouts. */
  at: number;
}

export interface AudioCheckReading {
  level: number;
  vadP: number;
  gate: GateState;
  route: AudioRouteKind;
  inputRate: number;
  phase: AudioCheckPhase;
  at: number;
}

export interface EngineError {
  code: ErrorCode;
  params: Record<string, string | number>;
  at: number;
}

export interface EngineData {
  info: IntercomInfo | null;
  compat: ApiCompat;
  /** Plugin-level status ("idle" until startEngine resolves). */
  status: "idle" | "starting" | "running";
  snapshot: Snapshot | null;
  /** Time (ms) the last snapshot arrived. */
  snapshotAt: number | null;
  lastTalkDenied: TalkDenial | null;
  route: AudioRouteKind | null;
  audioCheck: AudioCheckReading | null;
  lastError: EngineError | null;
}

export interface EngineState extends EngineData {
  setInfo(info: IntercomInfo): void;
  setStatus(status: EngineData["status"]): void;
  applySnapshot(snapshot: Snapshot, at?: number): void;
  talkDenied(reason: TalkDeniedReason, at?: number): void;
  clearTalkDenied(): void;
  routeChanged(route: AudioRouteKind): void;
  audioCheckSample(sample: Omit<AudioCheckReading, "at">, at?: number): void;
  clearAudioCheck(): void;
  error(code: ErrorCode, params: Record<string, string | number>, at?: number): void;
  reset(): void;
}

/** `ok` when the plugin's API major equals the app's (a minor difference is compatible). */
export function apiCompat(pluginApiVersion: string | undefined): ApiCompat {
  if (pluginApiVersion === undefined) return "unknown";
  const major = Number.parseInt(pluginApiVersion.split(".")[0] ?? "", 10);
  if (!Number.isFinite(major)) return "mismatch";
  return major === API_VERSION_MAJOR ? "ok" : "mismatch";
}

/** The app's own UI API version (for the mismatch banner). */
export const APP_API_VERSION: string = API_VERSION;

const initial: EngineData = {
  info: null,
  compat: "unknown",
  status: "idle",
  snapshot: null,
  snapshotAt: null,
  lastTalkDenied: null,
  route: null,
  audioCheck: null,
  lastError: null,
};

export const useEngineStore = create<EngineState>()((set) => ({
  ...initial,
  setInfo: (info) => set({ info, compat: apiCompat(info.apiVersion) }),
  setStatus: (status) => set({ status }),
  applySnapshot: (snapshot, at = Date.now()) =>
    set((state) => ({
      snapshot,
      snapshotAt: at,
      route: snapshot.audio.route,
      // An engine that reports idle/crashed is not running, whatever the plugin said.
      status:
        snapshot.engine === "idle" || snapshot.engine === "crashed"
          ? state.status === "starting"
            ? "starting"
            : "idle"
          : "running",
    })),
  talkDenied: (reason, at = Date.now()) => set({ lastTalkDenied: { reason, at } }),
  clearTalkDenied: () => set({ lastTalkDenied: null }),
  routeChanged: (route) => set({ route }),
  audioCheckSample: (sample, at = Date.now()) => set({ audioCheck: { ...sample, at } }),
  clearAudioCheck: () => set({ audioCheck: null }),
  error: (code, params, at = Date.now()) => set({ lastError: { code, params, at } }),
  reset: () => set(initial),
}));

/** Selector helpers for components (stable references, no derived objects). */
export const selectSession = (s: EngineState) => s.snapshot?.session ?? null;
export const selectIsLive = (s: EngineState) => s.snapshot?.session?.live ?? false;
export const selectEngineCrashed = (s: EngineState) => s.snapshot?.engine === "crashed";
