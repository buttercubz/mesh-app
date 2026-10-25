// Test helpers: a fake MeshIntercom plugin that records every call (contract C6 shape)
// and lets tests answer commands and push events.
import type { CommandResult, UiCommand, UiEvent } from "@meshapp/engine-api";
import type { MeshIntercomPlugin } from "@meshapp/capacitor-intercom";

export interface PluginCall {
  method: string;
  args: unknown;
}

export interface FakePlugin extends MeshIntercomPlugin {
  readonly calls: PluginCall[];
  /** Commands that reached `send`, parsed. */
  readonly sent: UiCommand[];
  /** Commands that reached `gestureCommand`, parsed. */
  readonly gestured: UiCommand[];
  /** Answer for the next commands (default `{ ok: true }`). */
  answer: (command: UiCommand) => CommandResult;
  emit(event: UiEvent | string): void;
  listenerCount(): number;
  failStart?: unknown;
}

export function createFakePlugin(): FakePlugin {
  const calls: PluginCall[] = [];
  const sent: UiCommand[] = [];
  const gestured: UiCommand[] = [];
  const listeners = new Set<(e: { json: string }) => void>();
  const record = (method: string, args?: unknown) => {
    calls.push({ method, args });
  };
  const plugin: FakePlugin = {
    calls,
    sent,
    gestured,
    answer: () => ({ ok: true }),
    async getInfo() {
      record("getInfo");
      return { platform: "web", engine: "platform-only", apiVersion: "1.2", protocolVersion: 1, appVersion: "0.1.0" };
    },
    async getStatus() {
      record("getStatus");
      return { engine: "idle", live: false, uiAttached: true };
    },
    async checkPermissions() {
      record("checkPermissions");
      return { microphone: "prompt", bluetooth: "granted", notifications: "prompt", camera: "prompt", location: "prompt", nearbyWifi: "granted" };
    },
    async requestPermissions(opts) {
      record("requestPermissions", opts);
      return { microphone: "granted", bluetooth: "granted", notifications: "granted", camera: "granted", location: "denied", nearbyWifi: "granted" };
    },
    startEngine(opts) {
      record("startEngine", opts);
      return plugin.failStart === undefined ? Promise.resolve() : Promise.reject(plugin.failStart);
    },
    gestureCommand(opts) {
      record("gestureCommand", opts);
      const cmd = JSON.parse(opts.command) as UiCommand;
      gestured.push(cmd);
      return Promise.resolve({ result: JSON.stringify(plugin.answer(cmd)) });
    },
    async goLive() {
      record("goLive");
    },
    async resumeMic() {
      record("resumeMic");
    },
    async stopEngine() {
      record("stopEngine");
    },
    send(opts) {
      record("send", opts);
      const cmd = JSON.parse(opts.command) as UiCommand;
      sent.push(cmd);
      return Promise.resolve({ result: JSON.stringify(plugin.answer(cmd)) });
    },
    async ptt(opts) {
      record("ptt", opts);
    },
    async setKeepScreenOn(opts) {
      record("setKeepScreenOn", opts);
    },
    async scanInviteQr() {
      record("scanInviteQr");
      return { cancelled: true };
    },
    async shareFile(opts) {
      record("shareFile", opts);
    },
    async openAppSettings() {
      record("openAppSettings");
    },
    async requestBatteryExemption() {
      record("requestBatteryExemption");
      return { granted: true };
    },
    async setNotificationCatalog(opts) {
      record("setNotificationCatalog", opts);
    },
    async diagStatus() {
      record("diagStatus");
      return { enabled: true, sessionId: "d-20261022-2004-7k2q", rideKey: "9f3a71c2", node: 4711, bytes: 12_400_000, sessions: 7, oldestMs: Date.now() - 3 * 86_400_000 };
    },
    async diagList() {
      record("diagList");
      return { sessions: [] };
    },
    async diagExport(opts) {
      record("diagExport", opts);
      return { path: "convlink-diag.zip", mime: "application/zip", sessions: opts.scope === "last" ? 1 : 7 };
    },
    async diagClear() {
      record("diagClear");
    },
    async diagSetEnabled(opts) {
      record("diagSetEnabled", opts);
    },
    async diagMark(opts) {
      record("diagMark", opts);
    },
    async awareStatus() {
      record("awareStatus");
      return { supported: true, enabled: false };
    },
    async awareSetEnabled(opts) {
      record("awareSetEnabled", opts);
    },
    async addListener(_event, cb) {
      record("addListener");
      listeners.add(cb);
      return {
        remove: async () => {
          listeners.delete(cb);
        },
      };
    },
    emit(event) {
      const json = typeof event === "string" ? event : JSON.stringify(event);
      for (const l of [...listeners]) l({ json });
    },
    listenerCount: () => listeners.size,
  };
  return plugin;
}

/** Waits for pending promise callbacks. */
export const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
