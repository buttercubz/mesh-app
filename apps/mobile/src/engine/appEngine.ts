// The app engine client implementation (WP-18, DESIGN §11.1, §11.3): the only way UI code talks to
// the MeshIntercom plugin. It wraps `createEngineClient` from @meshapp/engine-api, which
// validates every command/result/event and routes every ★ command of GESTURE_COMMANDS
// through `gestureCommand` (never `send`). Plugin calls are issued synchronously, so a
// call made from a tap handler stays inside the user gesture (§3.2).
import type { EngineClient, ResultOf, UiCommand, UiEvent, Unsubscribe } from "@meshapp/engine-api";
import { createEngineClient } from "@meshapp/engine-api";
import type {
  AwareStatus,
  DiagExportScope,
  DiagMarkLabel,
  DiagStatus,
  IntercomInfo,
  IntercomPermission,
  IntercomPermissionStatus,
  IntercomStatus,
  MeshIntercomPlugin,
  StartEngineOptions,
} from "@meshapp/capacitor-intercom";
import { MeshIntercom } from "@meshapp/capacitor-intercom";

import type { AppLanguage } from "../i18n/index";
import { currentLanguage } from "../i18n/index";
import { useEngineStore } from "../store/useEngineStore";
import { useSettingsStore } from "../store/useSettingsStore";
import type { SetPromptsCommand } from "./prompts";
import { setPromptsCommand } from "./prompts";

export interface AppEngineDeps {
  /** Current prompt language and the spoken-prompts setting. */
  promptSettings(): { language: AppLanguage; enabled: boolean };
  /** Engine status sink (default: useEngineStore). */
  setStatus(status: "idle" | "starting" | "running"): void;
  setInfo(info: IntercomInfo): void;
  /** Diagnostics for non-fatal failures (default: console.warn). */
  warn(message: string, error?: unknown): void;
  /** Wall clock for the PTT T8 stamp. */
  now(): number;
}

export interface AppEngine {
  /** Typed command; ★ commands go through gestureCommand automatically. */
  command<C extends UiCommand>(command: C): Promise<ResultOf<C>>;
  /** Raw validated event stream (the app subscribes once, in useEngineEvents). */
  onEvent(handler: (event: UiEvent) => void): Promise<Unsubscribe>;
  getInfo(): Promise<IntercomInfo>;
  getStatus(): Promise<IntercomStatus>;
  checkPermissions(): Promise<IntercomPermissionStatus>;
  requestPermissions(permissions?: IntercomPermission[]): Promise<IntercomPermissionStatus>;
  /**
   * Gesture required: starts radios + engine thread (audio stays off), then sends the
   * spoken-prompt templates (`setPrompts`) for the current language.
   */
  startEngine(options: StartEngineOptions): Promise<void>;
  stopEngine(): Promise<void>;
  /** ★ Go live (gesture). */
  goLive(): Promise<ResultOf<{ type: "goLive" }>>;
  /** ★ Resume the microphone after CaptureLost (gesture). */
  resumeMic(): Promise<ResultOf<{ type: "resumeMic" }>>;
  /** PTT fast path, stamped with the UI time for the T8 bench. */
  ptt(down: boolean, tUiMs?: number): Promise<void>;
  setKeepScreenOn(on: boolean): Promise<void>;
  scanInviteQr(): Promise<{ url?: string; cancelled: boolean }>;
  shareFile(path: string, mime: string): Promise<void>;
  openAppSettings(): Promise<void>;
  requestBatteryExemption(): Promise<{ granted: boolean }>;
  /** Localized title and notice/error templates for native local notifications (C6). */
  setNotificationCatalog(title: string, templates: Record<string, string>): Promise<void>;
  /** Re-sends the prompt templates (language or setting changed while running). */
  syncPrompts(): Promise<ResultOf<SetPromptsCommand>>;
  /** Diagnostics recorder (ridelog/1, C6 1.1): state, export, clear, toggle, marker. */
  diagStatus(): Promise<DiagStatus>;
  diagExport(scope: DiagExportScope): Promise<{ path: string; mime: string; sessions: number }>;
  diagClear(): Promise<void>;
  diagSetEnabled(enabled: boolean): Promise<void>;
  diagMark(label: DiagMarkLabel): Promise<void>;
  /** Wi-Fi Aware beta (Android, C6 1.2): device support + toggle, and the switch. */
  awareStatus(): Promise<AwareStatus>;
  awareSetEnabled(enabled: boolean): Promise<void>;
  /** The wrapped typed client (tests, diagnostics). */
  readonly client: EngineClient;
}

const defaultDeps: AppEngineDeps = {
  promptSettings: () => ({
    language: currentLanguage(),
    enabled: useSettingsStore.getState().spokenPrompts,
  }),
  setStatus: (status) => useEngineStore.getState().setStatus(status),
  setInfo: (info) => useEngineStore.getState().setInfo(info),
  warn: (message, error) => console.warn(`[engine] ${message}`, error),
  now: () => Date.now(),
};

export function createAppEngine(plugin: MeshIntercomPlugin, overrides: Partial<AppEngineDeps> = {}): AppEngine {
  const deps: AppEngineDeps = { ...defaultDeps, ...overrides };
  const client = createEngineClient(plugin, {
    onContractError: (error, raw) => deps.warn(`dropped event that breaks the UI contract: ${error.detail}`, raw),
  });

  const syncPrompts = () => {
    const { language, enabled } = deps.promptSettings();
    return client.command(setPromptsCommand(language, enabled));
  };

  return {
    client,
    command: (cmd) => client.command(cmd),
    onEvent: (handler) => client.onEvent(handler),
    getInfo: async () => {
      const info = await plugin.getInfo();
      deps.setInfo(info);
      return info;
    },
    getStatus: () => plugin.getStatus(),
    checkPermissions: () => plugin.checkPermissions(),
    requestPermissions: (permissions) =>
      plugin.requestPermissions(permissions === undefined ? undefined : { permissions }),
    startEngine: (options) => {
      // Synchronous plugin call first: still inside the tap that asked for it.
      const started = plugin.startEngine(options);
      deps.setStatus("starting");
      return started.then(
        async () => {
          deps.setStatus("running");
          try {
            const result = await syncPrompts();
            if (!result.ok) deps.warn(`setPrompts refused: ${result.code}`);
          } catch (e) {
            // Prompts are an addition to the earcons (§7.7): the ride goes on without them.
            deps.warn("setPrompts failed", e);
          }
        },
        (reason: unknown) => {
          deps.setStatus("idle");
          throw reason;
        },
      );
    },
    stopEngine: async () => {
      await plugin.stopEngine();
      deps.setStatus("idle");
    },
    goLive: () => client.command({ type: "goLive" }),
    resumeMic: () => client.command({ type: "resumeMic" }),
    ptt: (down, tUiMs) => plugin.ptt({ down, tUiMs: tUiMs ?? deps.now() }),
    setKeepScreenOn: (on) => plugin.setKeepScreenOn({ on }),
    scanInviteQr: () => plugin.scanInviteQr(),
    shareFile: (path, mime) => plugin.shareFile({ path, mime }),
    openAppSettings: () => plugin.openAppSettings(),
    requestBatteryExemption: () => plugin.requestBatteryExemption(),
    setNotificationCatalog: (title, templates) => plugin.setNotificationCatalog({ title, templates }),
    syncPrompts,
    diagStatus: () => plugin.diagStatus(),
    diagExport: (scope) => plugin.diagExport({ scope }),
    diagClear: () => plugin.diagClear(),
    diagSetEnabled: (enabled) => plugin.diagSetEnabled({ enabled }),
    diagMark: (label) => plugin.diagMark({ label }),
    awareStatus: () => plugin.awareStatus(),
    awareSetEnabled: (enabled) => plugin.awareSetEnabled({ enabled }),
  };
}

/** The app-wide engine client over the MeshIntercom plugin. */
export const engine: AppEngine = createAppEngine(MeshIntercom);
