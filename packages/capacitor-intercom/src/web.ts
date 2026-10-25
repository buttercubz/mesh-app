// Web implementation of MeshIntercom (DESIGN §11.2), loaded lazily by `index.ts`.
// It mirrors the native gating so UI code that would fail on a phone also fails here:
// `gestureCommand` (and `startEngine`) need a visible page (the web stand-in for
// foreground-active), and `send` rejects every ★ command of WP-03's GESTURE_COMMANDS
// with `useGestureCommand`. Commands and events are mesh-api JSON (contract C1).
//
// Phase 1: the engine is @meshapp/engine-web (mesh_wasm.wasm: the real Rust core in a
// simulated convoy, DESIGN §11.2), created by `createDefaultBackend`. This module is
// itself loaded lazily by `index.ts`, so native builds never load engine-web or the
// WASM. The Phase 0 scripted Lab peer (`FakeLabEngine`) stays available with
// `?engine=fake` (UI work without the WASM module) and for tests.
import { WebPlugin } from "@capacitor/core";
import type { CommandResult, ErrorCode, UiCommand, UiEvent } from "@meshapp/engine-api";
import { ContractError, parseUiCommand, requiresGesture } from "@meshapp/engine-api";
import { createWebEngineBackend } from "@meshapp/engine-web";

import type {
  AwareStatus,
  DiagExportScope,
  DiagMarkLabel,
  DiagSessionSummary,
  DiagStatus,
  IntercomInfo,
  IntercomPermission,
  IntercomPermissionStatus,
  IntercomStatus,
  MeshIntercomPlugin,
  StartEngineOptions,
} from "./definitions.js";
import { DIAG_MARK_LABELS, INTERCOM_PERMISSIONS, MESH_INTERCOM_EVENT } from "./definitions.js";
import { IntercomError } from "./errors.js";
import { API_VERSION, PROTOCOL_VERSION } from "./version.js";
import type { EngineBackend } from "./web/backend.js";
import type { Visibility, WakeLockHandle, WebEnvironment } from "./web/environment.js";
import { browserEnvironment } from "./web/environment.js";
import { FakeLabEngine } from "./web/fake/fake-lab-engine.js";

export type { EngineBackend, BackendFile, DiagBackend } from "./web/backend.js";
export type { SharedFile, Visibility, WakeLockHandle, WebEnvironment } from "./web/environment.js";
export { browserEnvironment } from "./web/environment.js";
export { FakeLabEngine } from "./web/fake/fake-lab-engine.js";
export type { FakeLabEngineOptions } from "./web/fake/fake-lab-engine.js";

export interface MeshIntercomWebOptions {
  /** Engine backend (default: `createDefaultBackend`). */
  backend?: EngineBackend;
  /** Browser services (default: `browserEnvironment()`). */
  environment?: WebEnvironment;
}

/** `true` when the page asks for the Phase 0 scripted Lab peer (`?engine=fake`). */
export function wantsFakeEngine(query: string): boolean {
  return new URLSearchParams(query).get("engine") === "fake";
}

/** `true` when the page asks for the simulator dev handle (`?simDebug=1`). */
export function wantsSimDebug(query: string): boolean {
  return new URLSearchParams(query).get("simDebug") === "1";
}

/** Defines `globalThis.__meshSim` (non-writable) as the plugin's simulator handle. */
function exposeSimulatorHandle(plugin: { readonly simulator: EngineBackend }): void {
  Object.defineProperty(globalThis, "__meshSim", {
    configurable: true,
    enumerable: false,
    get: () => plugin.simulator,
  });
}

/**
 * The backend used when none is given: the WASM simulator of @meshapp/engine-web, with
 * its scenario read from the page's query string (`?sim=chain5&loss=0.1&audio=1`), or
 * the Phase 0 fake Lab peer with `?engine=fake`.
 */
export function createDefaultBackend(environment: WebEnvironment): EngineBackend {
  const query = environment.query();
  if (wantsFakeEngine(query)) {
    return new FakeLabEngine({ now: () => environment.now(), appVersion: environment.appVersion() });
  }
  return createWebEngineBackend({
    query,
    makeError: (code, detail) => new IntercomError(code, detail),
  });
}

/** Malformed command text → `invalidCommand`, like the engine's own JSON check. */
function decodeCommand(text: unknown): { raw: Record<string, unknown> } | { error: CommandResult } {
  if (typeof text !== "string") return { error: { ok: false, code: "invalidCommand" } };
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (_e) {
    // Not JSON: the engine answers invalidCommand, so does the web implementation.
    return { error: { ok: false, code: "invalidCommand" } };
  }
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return { error: { ok: false, code: "invalidCommand" } };
  }
  return { raw: raw as Record<string, unknown> };
}

const answer = (result: CommandResult): { result: string } => ({ result: JSON.stringify(result) });

export class MeshIntercomWeb extends WebPlugin implements MeshIntercomPlugin {
  readonly #backend: EngineBackend;
  readonly #env: WebEnvironment;
  readonly #unsubscribeVisibility: () => void;
  #uiAttached: boolean;
  #wakeLock: WakeLockHandle | null = null;
  #keepScreenOn = false;

  constructor(options: MeshIntercomWebOptions = {}) {
    super();
    this.#env = options.environment ?? browserEnvironment();
    this.#backend = options.backend ?? createDefaultBackend(this.#env);
    this.#backend.setEventSink((event: UiEvent) => {
      this.notifyListeners(MESH_INTERCOM_EVENT, { json: JSON.stringify(event) });
    });
    this.#uiAttached = this.#env.visibility() === "visible";
    this.#backend.setUiAttached(this.#uiAttached);
    this.#unsubscribeVisibility = this.#env.onVisibilityChange((v) => this.#onVisibility(v));
    if (wantsSimDebug(this.#env.query())) exposeSimulatorHandle(this);
  }

  /**
   * Dev/test handle on the engine backend (read-only): with the WASM simulator it is the
   * `WebEngineBackend`, whose `convoy` runs `sim.*` commands (`sim.botTalk`,
   * `sim.botAway`, `sim.info`). Pages opened with `?simDebug=1` also get it as
   * `globalThis.__meshSim`. Never present in native builds (this module is web-only).
   */
  get simulator(): EngineBackend {
    return this.#backend;
  }

  /** `true` while a keep-screen-on request is in force (reset on every background). */
  get keepScreenOn(): boolean {
    return this.#keepScreenOn;
  }

  async getInfo(): Promise<IntercomInfo> {
    return {
      platform: "web",
      engine: this.#backend.engineKind,
      apiVersion: API_VERSION,
      protocolVersion: PROTOCOL_VERSION,
      appVersion: this.#env.appVersion(),
    };
  }

  async getStatus(): Promise<IntercomStatus> {
    return { ...this.#backend.status(), uiAttached: this.#uiAttached };
  }

  async checkPermissions(): Promise<IntercomPermissionStatus> {
    return this.#permissionStatus(new Map());
  }

  async requestPermissions(opts?: { permissions?: IntercomPermission[] }): Promise<IntercomPermissionStatus> {
    const wanted = opts?.permissions ?? [...INTERCOM_PERMISSIONS];
    const asked = new Map<IntercomPermission, IntercomPermissionStatus[IntercomPermission]>();
    // One prompt at a time, in the onboarding order.
    for (const p of INTERCOM_PERMISSIONS) {
      if (wanted.includes(p)) asked.set(p, await this.#env.requestPermission(p));
    }
    return this.#permissionStatus(asked);
  }

  async startEngine(opts: StartEngineOptions): Promise<void> {
    this.#requireForeground();
    await this.#backend.start(opts ?? {});
  }

  async gestureCommand(opts: { command: string }): Promise<{ result: string }> {
    // Checked synchronously at the call, i.e. inside the tap that issued it.
    this.#requireForeground();
    const decoded = decodeCommand(opts?.command);
    if ("error" in decoded) return answer(decoded.error);
    const command = this.#validate(decoded.raw);
    if ("ok" in command) return answer(command);
    return answer(await this.#recorded(command.cmd, true));
  }

  async goLive(): Promise<void> {
    await this.#sugar({ type: "goLive" });
  }

  async resumeMic(): Promise<void> {
    await this.#sugar({ type: "resumeMic" });
  }

  async stopEngine(): Promise<void> {
    await this.#backend.stop();
  }

  async send(opts: { command: string }): Promise<{ result: string }> {
    const decoded = decodeCommand(opts?.command);
    if ("error" in decoded) return answer(decoded.error);
    // Gate on the raw JSON first: a ★ command is refused even if it is malformed.
    if (requiresGesture(decoded.raw as unknown as UiCommand)) {
      throw new IntercomError("useGestureCommand");
    }
    const command = this.#validate(decoded.raw);
    if ("ok" in command) return answer(command);
    return answer(await this.#recorded(command.cmd, false));
  }

  async ptt(opts: { down: boolean; tUiMs?: number }): Promise<void> {
    const receivedAtMs = this.#env.now();
    if (typeof opts?.down !== "boolean") throw new IntercomError("invalidParams", "down");
    if (opts.tUiMs !== undefined && !Number.isFinite(opts.tUiMs)) {
      throw new IntercomError("invalidParams", "tUiMs");
    }
    if (this.#backend.status().engine !== "running") throw new IntercomError("engineNotRunning");
    // Fast path first (native: mesh_rt_set_flags), then the regular command. Like the
    // bridges, the command's answer is not surfaced: talk state arrives in snapshots
    // and refusals as `talkDenied` events.
    this.#backend.pttFast(opts.down, opts.tUiMs, receivedAtMs);
    await this.#backend.command({ type: "ptt", down: opts.down });
  }

  async setKeepScreenOn(opts: { on: boolean }): Promise<void> {
    if (typeof opts?.on !== "boolean") throw new IntercomError("invalidParams", "on");
    if (!opts.on) {
      this.#keepScreenOn = false;
      await this.#releaseWakeLock();
      return;
    }
    this.#keepScreenOn = true;
    if (this.#wakeLock === null) this.#wakeLock = await this.#env.requestWakeLock();
  }

  async scanInviteQr(): Promise<{ url?: string; cancelled: boolean }> {
    // The WASM backend loads in the background: wait for it instead of answering
    // `cancelled` to a fast tap right after page load.
    const loading = this.#backend as EngineBackend & { whenReady?: () => Promise<void> };
    if (typeof loading.whenReady === "function") await loading.whenReady();
    return this.#backend.scanInviteQr();
  }

  async shareFile(opts: { path: string; mime: string }): Promise<void> {
    const file = typeof opts?.path === "string" ? this.#backend.readFile(opts.path) : undefined;
    if (file === undefined) throw new IntercomError("invalidParams", "path");
    const shared = await this.#env.shareFile({
      name: file.name,
      data: file.data,
      mime: opts.mime === "" ? file.mime : opts.mime,
    });
    if (!shared) throw new IntercomError("notSupported", "no share sheet or download");
  }

  async setNotificationCatalog(opts: { title: string; templates: Record<string, string> }): Promise<void> {
    // Same validation as the iOS bridge; a browser posts no local notifications, so the
    // catalog is not kept (the UI shows notices itself while the page is visible).
    if (typeof opts?.title !== "string") throw new IntercomError("invalidParams", "title");
    const templates: unknown = opts.templates;
    if (typeof templates !== "object" || templates === null || Array.isArray(templates)) {
      throw new IntercomError("invalidParams", "templates");
    }
  }

  async openAppSettings(): Promise<void> {
    throw new IntercomError("notSupported", "no app settings page in a browser");
  }

  async requestBatteryExemption(): Promise<{ granted: boolean }> {
    // Android only; nothing to exempt in a browser.
    return { granted: true };
  }

  async diagStatus(): Promise<DiagStatus> {
    const d = this.#backend.diag;
    if (d === undefined) {
      return { enabled: false, sessionId: null, rideKey: null, node: null, bytes: 0, sessions: 0, oldestMs: null };
    }
    return d.status();
  }

  async diagList(): Promise<{ sessions: DiagSessionSummary[] }> {
    return { sessions: this.#backend.diag?.list() ?? [] };
  }

  async diagExport(opts: { scope: DiagExportScope }): Promise<{ path: string; mime: string; sessions: number }> {
    if (opts?.scope !== "last" && opts?.scope !== "all") throw new IntercomError("invalidParams", "scope");
    const files = this.#backend.diag?.files(opts.scope) ?? [];
    // A browser shares or downloads one file at a time: one `.jsonl` per session.
    for (const f of files) {
      const shared = await this.#env.shareFile({ name: f.name, data: f.data, mime: f.mime });
      if (!shared) throw new IntercomError("notSupported", "no share sheet or download");
    }
    const first = files[0];
    return { path: first?.name ?? "", mime: first?.mime ?? "", sessions: files.length };
  }

  async diagClear(): Promise<void> {
    this.#backend.diag?.clear();
  }

  async diagSetEnabled(opts: { enabled: boolean }): Promise<void> {
    if (typeof opts?.enabled !== "boolean") throw new IntercomError("invalidParams", "enabled");
    this.#backend.diag?.setEnabled(opts.enabled);
  }

  async diagMark(opts: { label: DiagMarkLabel }): Promise<void> {
    const label: unknown = opts?.label;
    if (typeof label !== "string" || !(DIAG_MARK_LABELS as readonly string[]).includes(label)) {
      throw new IntercomError("invalidParams", "label");
    }
    this.#backend.diag?.mark(label);
  }

  /** No Wi-Fi Aware in the browser (C6 1.2). */
  async awareStatus(): Promise<AwareStatus> {
    return { supported: false, enabled: false };
  }

  async awareSetEnabled(opts: { enabled: boolean }): Promise<void> {
    if (typeof opts?.enabled !== "boolean") throw new IntercomError("invalidParams", "enabled");
    throw new IntercomError("notSupported", "wifiAware");
  }

  /** Stops the engine and detaches from the page (tests, hot reload). */
  async dispose(): Promise<void> {
    this.#unsubscribeVisibility();
    await this.#releaseWakeLock();
    await this.#backend.stop();
    await this.removeAllListeners();
  }

  /**
   * Runs a validated command and records it as a `cmd` diagnostics note (command id,
   * gesture, answer code, round trip; never the parameters).
   */
  async #recorded(cmd: UiCommand, gesture: boolean): Promise<CommandResult> {
    const t0 = this.#env.now();
    const r = await this.#backend.command(cmd);
    // Snapshot requests are polling, not user actions.
    if (cmd.type === "requestSnapshot") return r;
    this.#backend.diag?.note("cmd", {
      name: cmd.type,
      gesture,
      ok: r.ok,
      err: r.ok ? null : r.code,
      ms: Math.max(0, Math.round(this.#env.now() - t0)),
    });
    return r;
  }

  #requireForeground(): void {
    if (this.#env.visibility() !== "visible") throw new IntercomError("notForeground");
  }

  #validate(raw: Record<string, unknown>): { cmd: UiCommand } | CommandResult {
    try {
      return { cmd: parseUiCommand(raw) };
    } catch (e) {
      // Contract violations answer invalidCommand; anything else is a bug.
      if (!(e instanceof ContractError)) throw e;
      return { ok: false, code: "invalidCommand" };
    }
  }

  async #sugar(command: UiCommand): Promise<void> {
    const { result } = await this.gestureCommand({ command: JSON.stringify(command) });
    const parsed = JSON.parse(result) as CommandResult;
    if (!parsed.ok) throw new IntercomError(parsed.code satisfies ErrorCode);
  }

  async #permissionStatus(
    known: Map<IntercomPermission, IntercomPermissionStatus[IntercomPermission]>,
  ): Promise<IntercomPermissionStatus> {
    const entries = await Promise.all(
      INTERCOM_PERMISSIONS.map(async (p) => [p, known.get(p) ?? (await this.#env.queryPermission(p))] as const),
    );
    return Object.fromEntries(entries) as IntercomPermissionStatus;
  }

  #onVisibility(visibility: Visibility): void {
    const attached = visibility === "visible";
    if (attached === this.#uiAttached) return;
    this.#uiAttached = attached;
    this.#backend.setUiAttached(attached);
    if (!attached) {
      // Keep-screen-on is reset on every background transition (§11.1, WP-19 contract).
      this.#keepScreenOn = false;
      void this.#releaseWakeLock();
    }
  }

  async #releaseWakeLock(): Promise<void> {
    const lock = this.#wakeLock;
    this.#wakeLock = null;
    if (lock === null) return;
    try {
      await lock.release();
    } catch (_e) {
      // Already released by the browser (page hidden): nothing left to do.
    }
  }
}
