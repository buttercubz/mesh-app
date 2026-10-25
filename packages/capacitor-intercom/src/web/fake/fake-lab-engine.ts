// Phase 0 web backend: a scripted fake Lab peer (WP-17, DESIGN §11.2, §12.5). After
// `startEngine({ lab: { code } })` one fake phone joins the Lab code, Lab runs produce
// live numbers, summaries and labrun/1 files, and audio checks emit meter samples.
// There is no group crypto here: group commands answer `notSupported`/`noSession`
// until the WASM backend (WP-13, Phase 1) replaces this class.
import type { CommandResult, UiCommand, UiEvent } from "@meshapp/engine-api";
import { DISPLAY_NAME_MAX_BYTES } from "@meshapp/engine-api";

import type { StartEngineOptions } from "../../definitions.js";
import { IntercomError } from "../../errors.js";
import type { BackendFile, EngineBackend } from "../backend.js";
import { AudioCheckRunner } from "./audio-check.js";
import { LabController } from "./lab-controller.js";
import { hashSeed, Rng } from "./rng.js";
import { fakeSnapshot } from "./snapshot.js";

/** Engine tick (20 Hz: audio-check samples; Lab runs sample at 1 Hz, progress at 2 Hz). */
export const FAKE_TICK_MS = 50;
/** Delay before the scripted peer joins the Lab code. */
export const FAKE_PEER_JOIN_MS = 1500;
/** Minimum snapshot period (≤ 10 Hz, only when changed). */
export const SNAPSHOT_MIN_MS = 100;

export interface FakeLabEngineOptions {
  /** Clock in ms (default `Date.now`). */
  now?: () => number;
  /** Seed salt (default: the Lab code). */
  seed?: string;
  appVersion?: string;
}

const LAB_CODE = /^\d{4}$/;

const err = (code: Extract<CommandResult, { ok: false }>["code"]): CommandResult => ({ ok: false, code });

/** Commands answered without a running engine (persisted settings and stored data). */
const IDLE_OK: ReadonlySet<UiCommand["type"]> = new Set([
  "parseInvite",
  "listGroups",
  "forgetGroup",
  "setDisplayName",
  "setVehicle",
  "setPttStyle",
  "setVoxSensitivity",
  "setRoutePreference",
  "setShareDistance",
  "setPrompts",
  "setBuiltinMicTx",
  "labListRuns",
  "labDeleteRun",
]);

export class FakeLabEngine implements EngineBackend {
  readonly engineKind = "platform-only" as const;
  readonly #now: () => number;
  readonly #seed: string | undefined;
  readonly #appVersion: string;
  #sink: (event: UiEvent) => void = () => {};
  #rng: Rng;
  #lab: LabController;
  #audioCheck: AudioCheckRunner;
  #timer: ReturnType<typeof setInterval> | null = null;
  #running = false;
  #labCode: string | null = null;
  #startedMs = 0;
  #peer: number | null = null;
  #uiAttached = true;
  #lastSnapshot = "";
  #lastSnapshotMs = -Infinity;
  #prefs = new Map<string, unknown>();
  #lastPtt: { down: boolean; tUiMs: number | undefined; receivedAtMs: number } | null = null;

  constructor(options: FakeLabEngineOptions = {}) {
    this.#now = options.now ?? (() => Date.now());
    this.#seed = options.seed;
    this.#appVersion = options.appVersion ?? "0.0.0-web";
    this.#rng = new Rng(hashSeed(this.#seed ?? "web"));
    this.#lab = this.#newLab();
    this.#audioCheck = new AudioCheckRunner(this.#rng);
  }

  setEventSink(sink: (event: UiEvent) => void): void {
    this.#sink = sink;
  }

  async start(opts: StartEngineOptions): Promise<void> {
    const modes = [opts.groupId !== undefined, opts.lab !== undefined, opts.demo === true];
    if (modes.filter(Boolean).length > 1) throw new IntercomError("invalidParams", "one of groupId, lab, demo");
    if (opts.demo === true) throw new IntercomError("notSupported", "Demo ride needs the WASM engine (WP-13)");
    if (opts.groupId !== undefined) throw new IntercomError("unknownGroup");
    if (opts.lab !== undefined && !LAB_CODE.test(opts.lab.code)) {
      throw new IntercomError("invalidParams", "lab code is 4 digits");
    }
    if (this.#running) await this.stop();
    const code = opts.lab?.code ?? null;
    // Lab runs survive an engine restart in the same page (files are kept), like the
    // phone's Documents/lab; only the random generator is re-seeded per Lab code.
    this.#rng.reseed(hashSeed(`${this.#seed ?? ""}|${code ?? "none"}`));
    this.#running = true;
    this.#labCode = code;
    this.#startedMs = this.#now();
    this.#peer = null;
    this.#timer = setInterval(() => this.tick(), FAKE_TICK_MS);
    this.#emitSnapshot(true);
  }

  async stop(): Promise<void> {
    if (!this.#running) return;
    const now = this.#now();
    this.#lab.abort(now);
    this.#audioCheck.stop();
    if (this.#timer !== null) clearInterval(this.#timer);
    this.#timer = null;
    this.#running = false;
    this.#labCode = null;
    this.#peer = null;
    this.#emitSnapshot(true);
  }

  status(): { engine: "idle" | "running"; live: boolean } {
    return { engine: this.#running ? "running" : "idle", live: false };
  }

  setUiAttached(attached: boolean): void {
    this.#uiAttached = attached;
    // Foreground return: a full snapshot right away (bridges do the same, §11.1).
    if (attached) this.#emitSnapshot(true);
  }

  pttFast(down: boolean, tUiMs: number | undefined, receivedAtMs: number): void {
    this.#lastPtt = { down, tUiMs, receivedAtMs };
  }

  /** Last PTT fast-path receipt (T8 stamp), for diagnostics and tests. */
  get lastPtt(): { down: boolean; tUiMs: number | undefined; receivedAtMs: number } | null {
    return this.#lastPtt;
  }

  /** Stored value of a settings command (diagnostics and tests). */
  pref(name: string): unknown {
    return this.#prefs.get(name);
  }

  readFile(path: string): BackendFile | undefined {
    return this.#lab.readFile(path);
  }

  scanInviteQr(): { url?: string; cancelled: boolean } {
    // No group crypto in the Phase 0 fake: behave like a cancelled scan.
    return { cancelled: true };
  }

  command(cmd: UiCommand): CommandResult {
    if (!this.#running && !IDLE_OK.has(cmd.type)) return err("engineNotRunning");
    const now = this.#now();
    switch (cmd.type) {
      case "createGroup":
      case "joinGroup":
      case "parseInvite":
      case "joinOpenChannel":
      case "setMemberMuted":
      case "setMemberBlocked":
        return err("notSupported");
      case "listGroups":
        return { ok: true, groups: [] };
      case "forgetGroup":
      case "getInvite":
        return err("unknownGroup");
      case "goLive":
      case "resumeMic":
      case "setTalkMode":
      case "ptt":
      case "rideControl":
      case "removeMember":
      case "lockGroup":
      case "closeGroup":
      case "previewAccessLoss":
      case "transferLeader":
      case "setInviteExpiry":
        return err("noSession");
      case "setDisplayName": {
        const bytes = new TextEncoder().encode(cmd.name.trim()).length;
        if (bytes === 0 || bytes > DISPLAY_NAME_MAX_BYTES) return err("invalidParams");
        return this.#setPref("displayName", cmd.name.trim());
      }
      case "setVehicle":
        return this.#setPref("vehicle", cmd.vehicle);
      case "setPttStyle":
        return this.#setPref("pttStyle", cmd.style);
      case "setVoxSensitivity":
        return this.#setPref("voxSensitivity", cmd.level);
      case "setRoutePreference":
        return this.#setPref("routePreference", cmd.pref);
      case "setShareDistance":
        return this.#setPref("shareDistance", cmd.enabled);
      case "setPrompts":
        return this.#setPref(`prompts.${cmd.lang}`, { enabled: cmd.enabled, templates: cmd.templates });
      case "setBuiltinMicTx":
        return this.#setPref("builtinMicTx", cmd.allowed);
      case "audioCheck":
        if (cmd.op === "stop") {
          this.#audioCheck.stop();
          return { ok: true };
        }
        if (this.#lab.activeAudio !== null) return err("wrongMode");
        this.#audioCheck.start(cmd.mode, now);
        return { ok: true };
      case "requestSnapshot":
        this.#emitSnapshot(true);
        return { ok: true };
      case "labStart":
        if (this.#labCode === null) return err("wrongMode");
        if (this.#audioCheck.active) return err("wrongMode");
        return this.#lab.start(cmd.bench, cmd.params, now);
      case "labStop":
        return this.#lab.stop(now);
      case "labMark":
        return this.#lab.mark(cmd.label, now);
      case "labListRuns":
        return this.#lab.list();
      case "labDeleteRun":
        return this.#lab.delete(cmd.runId);
    }
  }

  /** One engine step; driven by the interval timer (public for deterministic tests). */
  tick(): void {
    if (!this.#running) return;
    const now = this.#now();
    if (this.#labCode !== null && this.#peer === null && now - this.#startedMs >= FAKE_PEER_JOIN_MS) {
      this.#peer = this.#rng.int(1, 0xfffe);
      this.#sink({ type: "notice", key: "labPeerJoined", params: { labNode: this.#peer } });
      this.#lab.peerChanged(true, this.#peer, now);
    }
    this.#lab.tick(now);
    const sample = this.#audioCheck.tick(now);
    if (sample !== null) this.#sink(sample);
    this.#emitSnapshot(false);
  }

  #setPref(name: string, value: unknown): CommandResult {
    this.#prefs.set(name, value);
    return { ok: true };
  }

  #newLab(): LabController {
    return new LabController({
      emit: (event) => this.#sink(event),
      rng: this.#rng,
      appVersion: this.#appVersion,
      peerLabNode: () => this.#peer,
      uiAttached: () => this.#uiAttached,
    });
  }

  #emitSnapshot(force: boolean): void {
    if (!this.#uiAttached) return;
    const now = this.#now();
    if (!force && now - this.#lastSnapshotMs < SNAPSHOT_MIN_MS) return;
    const labAudio = this.#lab.activeAudio;
    const s = fakeSnapshot({
      running: this.#running,
      uiAttached: this.#uiAttached,
      peers: this.#peer === null ? 0 : 1,
      render: (labAudio !== null && labAudio !== "off") || this.#audioCheck.active,
      capture: labAudio === "full" || this.#audioCheck.capturing,
      inputLevel: this.#audioCheck.level,
    });
    const json = JSON.stringify(s);
    if (!force && json === this.#lastSnapshot) return;
    this.#lastSnapshot = json;
    this.#lastSnapshotMs = now;
    this.#sink({ type: "snapshot", s });
  }
}
