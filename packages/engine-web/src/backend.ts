// The engine behind the MeshIntercom web implementation (DESIGN §11.2): it matches the
// `EngineBackend` interface of @meshapp/capacitor-intercom (`src/web/backend.ts`)
// structurally, so the plugin's `createDefaultBackend` can return it while gating,
// visibility, wake lock, permissions and sharing stay in the plugin. This package does
// not import the plugin (the plugin loads this package lazily).
//
// The world is created on construction (the module loads in the background) and lives
// as long as the page: groups created or joined stay in memory only, never in web
// storage. `start`/`stop` map to the platform's engine start/stop (`sim.startEngine` /
// `sim.stopEngine`); the 20 ms clock runs while the engine runs.
import type { CommandResult, ErrorCode, UiCommand, UiEvent } from "@meshapp/engine-api";

import { TalkerNoisePlayer } from "./audio.js";
import { RideLogRecorder, groupRideKey, labRideKey } from "./diag-files.js";
import type { SimConvoy, SimConvoyOptions } from "./convoy.js";
import { createSimConvoy } from "./convoy.js";
import { scenarioFromQuery } from "./scenario.js";

/** `startEngine` options (contract C6). */
export interface StartEngineOptions {
  groupId?: string;
  lab?: { code: string };
  demo?: boolean;
}

/** A file the engine wrote, for `shareFile`. */
export interface BackendFile {
  name: string;
  data: string;
  mime: string;
}

/** A rejection carrying a contract `ErrorCode` as `code` and `message` (what
 *  engine-api's `rejectionCode()` reads, like the plugin's `IntercomError`). */
export class EngineWebError extends Error {
  readonly code: ErrorCode;
  readonly detail: string | undefined;

  constructor(code: ErrorCode, detail?: string) {
    super(code);
    this.name = "EngineWebError";
    this.code = code;
    this.detail = detail;
  }
}

export interface WebEngineBackendOptions extends SimConvoyOptions {
  /** Builds rejections (the plugin passes its `IntercomError`). */
  makeError?: (code: ErrorCode, detail?: string) => Error;
  /** App version written in the diagnostics header. */
  appVersion?: string;
  /** Wall clock for diagnostics (default `Date.now`). */
  wallNow?: () => number;
}

/** The PTT fast-path stamp of the last press (T8, §12.2). */
export interface PttStamp {
  down: boolean;
  tUiMs: number | undefined;
  receivedAtMs: number;
}

export class WebEngineBackend {
  readonly engineKind = "full" as const;
  readonly #ready: Promise<SimConvoy>;
  readonly #makeError: (code: ErrorCode, detail?: string) => Error;
  readonly #audio: TalkerNoisePlayer | undefined;
  #convoy: SimConvoy | undefined;
  #sink: ((event: UiEvent) => void) | undefined;
  #attached = true;
  #running = false;
  #lastPtt: PttStamp | undefined;
  /** The simulated phone's ridelog/1 recorder (in memory only). */
  readonly diag: RideLogRecorder;
  readonly #wallNow: () => number;

  constructor(opts: WebEngineBackendOptions = {}) {
    this.#wallNow = opts.wallNow ?? Date.now;
    this.diag = new RideLogRecorder(
      { model: "web-sim", os: "browser", app: opts.appVersion ?? "0.0.0-web" },
      this.#wallNow,
    );
    this.#makeError = opts.makeError ?? ((code, detail) => new EngineWebError(code, detail));
    // `?audio=1` turns on speech-like noise for remote talkers (when WebAudio exists).
    const wantAudio = opts.query !== undefined && scenarioFromQuery(opts.query).page.audio;
    this.#audio = opts.audio ?? (wantAudio ? TalkerNoisePlayer.create() : undefined);
    this.#ready = createSimConvoy({ ...opts, audio: this.#audio }).then((c) => {
      this.#convoy = c;
      c.onEvent((e) => {
        this.diag.uiEvent(e);
        this.#sink?.(e);
      });
      c.run({ type: "sim.uiAttached", attached: this.#attached });
      return c;
    });
    // Load failures surface on the first call that needs the engine.
    this.#ready.catch((_e: unknown) => undefined);
  }

  /** Resolves once the module is loaded and the world exists. */
  ready(): Promise<SimConvoy> {
    return this.#ready;
  }

  /** The convoy, once loaded (dev tools drive bots through its `sim.*` commands). */
  get convoy(): SimConvoy | undefined {
    return this.#convoy;
  }

  /** The last PTT fast-path stamp. */
  get lastPtt(): PttStamp | undefined {
    return this.#lastPtt;
  }

  setEventSink(sink: (event: UiEvent) => void): void {
    this.#sink = sink;
  }

  async start(opts: StartEngineOptions): Promise<void> {
    // Still inside the tap: browsers only let a gesture resume an AudioContext.
    this.#audio?.resume().catch((_e: unknown) => {
      // Autoplay refused: the simulator runs silently, which is fine.
    });
    const c = await this.#engine();
    if (this.#running) {
      c.run({ type: "sim.stopEngine" });
      this.#running = false;
    }
    const r = c.run({ type: "sim.startEngine", ...opts });
    if (!r.ok) throw this.#makeError(r.code, "startEngine");
    this.#running = true;
    const [mode, key] = await diagIdentity(opts, this.#wallNow());
    this.diag.begin(mode, key);
    c.start();
  }

  async stop(): Promise<void> {
    const c = this.#convoy;
    if (c === undefined || !this.#running) return;
    c.run({ type: "sim.stopEngine" });
    this.#running = false;
    this.diag.end();
    c.stop();
  }

  async command(command: UiCommand): Promise<CommandResult> {
    const c = await this.#engine();
    const r = c.run(command);
    if (command.type === "labDeleteRun" && r.ok) c.forgetRun(command.runId);
    return r;
  }

  pttFast(down: boolean, tUiMs: number | undefined, receivedAtMs: number): void {
    this.#lastPtt = { down, tUiMs, receivedAtMs };
    this.#convoy?.run({ type: "sim.ptt", down });
  }

  setUiAttached(attached: boolean): void {
    this.#attached = attached;
    this.#convoy?.run({ type: "sim.uiAttached", attached });
  }

  status(): { engine: "idle" | "running"; live: boolean } {
    const live = this.#running && (this.#convoy?.snapshot?.session?.live ?? false);
    return { engine: this.#running ? "running" : "idle", live };
  }

  readFile(path: string): BackendFile | undefined {
    return this.#convoy?.readFile(path);
  }

  /** Resolves once the WASM world exists (rejects with `internal` when it failed to
   *  load). The plugin awaits it before `scanInviteQr`, so a fast tap after page load
   *  still gets the invite. */
  async whenReady(): Promise<void> {
    await this.#engine();
  }

  /** The sim invite: the ride the bots are in (`cancelled` only before `whenReady`). */
  scanInviteQr(): { url?: string; cancelled: boolean } {
    const r = this.#convoy?.run({ type: "sim.invite" });
    if (r?.ok === true && typeof r["url"] === "string") return { url: r["url"], cancelled: false };
    return { cancelled: true };
  }

  /** Stops the engine and frees the world. */
  async dispose(): Promise<void> {
    await this.stop();
    this.#convoy?.dispose();
  }

  async #engine(): Promise<SimConvoy> {
    try {
      return await this.#ready;
    } catch (e) {
      throw this.#makeError("internal", e instanceof Error ? e.message : String(e));
    }
  }
}

/** Diagnostics mode and ride key of a start (the same keys the native runtime writes). */
async function diagIdentity(opts: StartEngineOptions, wallMs: number): Promise<[string, string | null]> {
  try {
    if (opts.demo === true) return ["demo", "demo"];
    if (opts.lab !== undefined) return ["lab", await labRideKey(Number(opts.lab.code))];
    if (opts.groupId !== undefined) return ["group", await groupRideKey(opts.groupId, wallMs)];
  } catch (_e) {
    // No WebCrypto (old browser, insecure origin): the session is recorded without a key.
  }
  return ["group", null];
}

/** A backend for the MeshIntercom web implementation (see `WebEngineBackend`). */
export function createWebEngineBackend(opts: WebEngineBackendOptions = {}): WebEngineBackend {
  return new WebEngineBackend(opts);
}
