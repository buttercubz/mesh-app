// Contract C6 (owner WP-17, DESIGN §11.1): the MeshIntercom Capacitor plugin API.
// Frozen per phase; any change goes through a contract change request to WP-17.
//
// Commands and events cross this boundary as mesh-api JSON text (contract C1,
// @meshapp/engine-api): `send`/`gestureCommand` take a serialized `UiCommand` and
// resolve a serialized `CommandResult`; the `event` stream delivers serialized
// `UiEvent`s. Rejections carry an `ErrorCode` of C1 as `code` (and `message`), e.g.
// `useGestureCommand`, `notForeground`, `engineNotRunning`.
import type { PermissionState, PluginListenerHandle } from "@capacitor/core";

export type { PermissionState, PluginListenerHandle } from "@capacitor/core";

/** Runtime permissions the intercom asks for (onboarding walkthrough, §11.4). */
export type IntercomPermission =
  | "microphone"
  | "bluetooth"
  | "notifications"
  | "camera"
  | "location"
  | "nearbyWifi";

/** Every `IntercomPermission`, in declaration order. */
export const INTERCOM_PERMISSIONS = [
  "microphone",
  "bluetooth",
  "notifications",
  "camera",
  "location",
  "nearbyWifi",
] as const satisfies readonly IntercomPermission[];

/** State of every intercom permission (`granted`, `denied`, `prompt`, `prompt-with-rationale`). */
export type IntercomPermissionStatus = Record<IntercomPermission, PermissionState>;

/** `getInfo()` answer. */
export interface IntercomInfo {
  platform: "ios" | "android" | "web";
  /** `platform-only` when the Rust engine is not linked (Swift-only debugging, web fake). */
  engine: "platform-only" | "full";
  /** UI contract version (`API_VERSION`, "1.2"); the UI refuses a different major. */
  apiVersion: string;
  /** Wire protocol version (Appendix A: 1). */
  protocolVersion: number;
  appVersion: string;
}

/** `getStatus()` answer. */
export interface IntercomStatus {
  engine: "idle" | "running";
  /** The local rider is live (Go live done, capture path open). */
  live: boolean;
  /** Snapshots are being emitted (app foreground with the WebView attached). */
  uiAttached: boolean;
}

/** `startEngine()` options: a group session, a Lab run (4-digit code) or the Demo ride. */
export interface StartEngineOptions {
  groupId?: string;
  lab?: { code: string };
  demo?: boolean;
}

/** Serialized mesh-api `UiCommand` (C1). */
export interface CommandOptions {
  command: string;
}

/** Serialized mesh-api `CommandResult` (C1). */
export interface CommandAnswer {
  result: string;
}

/**
 * `setNotificationCatalog()` options: the UI's localized strings for local
 * notifications posted by the bridge (capture lost, ...). `templates` maps an engine
 * `notices`/`errors` i18n key to its text in the current language (i18next `{{param}}`
 * templates, filled from the notice parameters). Sent again on every language change.
 */
export interface NotificationCatalog {
  /** Notification title (app name in the current language). */
  title: string;
  templates: Record<string, string>;
}

/** Fixed labels of a diagnostics marker (chips; never free text, no text chat). */
export type DiagMarkLabel = "crackle" | "cantHear" | "notHeard" | "lag" | "dropped" | "other";

/** Every `DiagMarkLabel`, in chip order. */
export const DIAG_MARK_LABELS = [
  "crackle",
  "cantHear",
  "notHeard",
  "lag",
  "dropped",
  "other",
] as const satisfies readonly DiagMarkLabel[];

/**
 * `diagStatus()` answer: the ridelog/1 diagnostics flight recorder (technical
 * connection and audio data; no voice, exact location, names or keys).
 */
export interface DiagStatus {
  /** Files are being written (the in-memory ring always records). */
  enabled: boolean;
  /** Open recording session (`d-YYYYMMDD-HHMM-xxxx`), if any. */
  sessionId: string | null;
  /** Ride key of that session (`9f3a71c2`, `lab-1a2b`, `demo`), if known. */
  rideKey: string | null;
  /** Group node id of this phone in that session, once known. */
  node: number | null;
  /** Stored bytes, sessions and the oldest session's time (Unix ms). */
  bytes: number;
  sessions: number;
  oldestMs: number | null;
}

/** One stored recording session (`diagList()`). */
export interface DiagSessionSummary {
  sessionId: string;
  bytes: number;
  segments: number;
  modifiedMs: number;
}

/** `diagExport()` scope: the most recent session, or every stored session. */
export type DiagExportScope = "last" | "all";

/** `awareStatus()` answer (C6 1.2). */
export interface AwareStatus {
  /** The device has Wi-Fi Aware (Android FEATURE_WIFI_AWARE). */
  supported: boolean;
  /** The user turned the beta on (off by default). */
  enabled: boolean;
}

/** One `event` notification: a serialized mesh-api `UiEvent` (C1). */
export interface IntercomEvent {
  json: string;
}

export interface MeshIntercomPlugin {
  getInfo(): Promise<{
    platform: "ios" | "android" | "web";
    engine: "platform-only" | "full";
    apiVersion: string;
    protocolVersion: number;
    appVersion: string;
  }>;
  getStatus(): Promise<{ engine: "idle" | "running"; live: boolean; uiAttached: boolean }>;
  checkPermissions(): Promise<IntercomPermissionStatus>;
  requestPermissions(opts?: { permissions?: IntercomPermission[] }): Promise<IntercomPermissionStatus>;
  /** Gesture required. Radios + engine thread; Android FGS (connectedDevice). Audio stays off. */
  startEngine(opts: { groupId?: string; lab?: { code: string }; demo?: boolean }): Promise<void>;
  /**
   * Gesture required; rejects with "notForeground" unless the app is foreground-active. For the ★
   * commands of §3.7 (goLive, resumeMic, labStart with audio "full"/"renderOnly", audioCheck start).
   * The bridge synchronously does the OS-level work (iOS: activate session, start VPIO with the RT
   * sink muted, location anchor if requested, Live Activity; Android: startForegroundService with
   * microphone [+location], open Oboe streams), then forwards the command; the core's StartCapture
   * only connects and unmutes the sink (§3.2).
   */
  gestureCommand(opts: { command: string }): Promise<{ result: string }>;
  /** Sugar for gestureCommand({type:"goLive"}) / ({type:"resumeMic"}). */
  goLive(): Promise<void>;
  resumeMic(): Promise<void>;
  stopEngine(): Promise<void>;
  /** JSON pass-through of mesh-api UiCommand → CommandResult; rejects ★ commands with "useGestureCommand". */
  send(opts: { command: string }): Promise<{ result: string }>;
  /** Fast path: writes mesh_rt_set_flags directly and also sends {type:"ptt"}. Stamps the native receive time for T8. */
  ptt(opts: { down: boolean; tUiMs?: number }): Promise<void>;
  /** iOS isIdleTimerDisabled / Android FLAG_KEEP_SCREEN_ON; reset automatically on background. */
  setKeepScreenOn(opts: { on: boolean }): Promise<void>;
  scanInviteQr(): Promise<{ url?: string; cancelled: boolean }>;
  /** Lab export via share sheet. */
  shareFile(opts: { path: string; mime: string }): Promise<void>;
  openAppSettings(): Promise<void>;
  /** Android only; no-op elsewhere. */
  requestBatteryExemption(): Promise<{ granted: boolean }>;
  /**
   * Localized strings for the bridge's local notifications (iOS NotifyAdapter; Android from
   * Phase 3). Until the UI sends one the bridge uses a built-in fallback. Rejects
   * `invalidParams` without `title`/`templates`. Web: validated, then a no-op.
   */
  setNotificationCatalog(opts: { title: string; templates: Record<string, string> }): Promise<void>;
  /**
   * Diagnostics recorder (ridelog/1) state for Settings → Diagnostics. Added in C6 1.1
   * (diagnostics flight recorder; every `diag*` method below came with it).
   */
  diagStatus(): Promise<DiagStatus>;
  /** Stored recording sessions, newest first. */
  diagList(): Promise<{ sessions: DiagSessionSummary[] }>;
  /**
   * Packs the last session (or all) plus `device.json` into one zip
   * (`convlink-diag-<rideKey>-n<node>-<date>.zip`; web: one `.jsonl` per session) and
   * opens the share sheet with it. Without stored files the in-memory ring is exported.
   * Resolves `sessions: 0` with `path: ""` when there is nothing to export.
   */
  diagExport(opts: { scope: DiagExportScope }): Promise<{ path: string; mime: string; sessions: number }>;
  /** Deletes every diagnostics file (the toggle stays as it is). */
  diagClear(): Promise<void>;
  /** Recording on/off; persisted natively (a plain flag, never a secret). */
  diagSetEnabled(opts: { enabled: boolean }): Promise<void>;
  /** A user marker ("problem now") with a fixed label; rejects `invalidParams` otherwise. */
  diagMark(opts: { label: DiagMarkLabel }): Promise<void>;
  /**
   * Wi-Fi Aware beta (Android↔Android link upgrade, WP-24). Added in C6 1.2. `supported`
   * is false on iOS and the web; the toggle is persisted natively (a plain flag).
   */
  awareStatus(): Promise<AwareStatus>;
  /** Turns the Wi-Fi Aware beta on/off; rejects `notSupported` where Aware is unsupported. */
  awareSetEnabled(opts: { enabled: boolean }): Promise<void>;
  /** UiEvent stream. */
  addListener(event: "event", cb: (e: { json: string }) => void): Promise<PluginListenerHandle>;
}

/**
 * Version of this plugin API (contract C6). 1.0: Phase 0-1 surface; 1.1: the `diag*`
 * methods (diagnostics flight recorder); 1.2: `awareStatus` / `awareSetEnabled` (Wi-Fi
 * Aware beta, Android). Additive minor bumps only within a major.
 */
export const PLUGIN_API_VERSION = "1.2";

/** Capacitor jsName of the plugin (native bridges register under the same name). */
export const MESH_INTERCOM_JS_NAME = "MeshIntercom";

/** Name of the single event the plugin emits. */
export const MESH_INTERCOM_EVENT = "event";
