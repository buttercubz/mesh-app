// The SimConvoy driver (DESIGN §11.2): one mesh-sim world inside mesh_wasm.wasm (node 0
// is this page's phone, the others are bots), advanced every 20 ms by the page clock.
// Commands are mesh-api JSON (contract C1) plus the simulator's `sim.*` commands;
// events are the local phone's UiEvents, delivered to every listener.
import type { CommandResult, Snapshot, UiEvent } from "@meshapp/engine-api";

import type { MeshWasm } from "./abi.js";
import { MeshWasmError } from "./abi.js";
import type { TalkerNoisePlayer } from "./audio.js";
import type { LabFile } from "./lab-files.js";
import { LabFileStore } from "./lab-files.js";
import type { LoadOptions } from "./loader.js";
import { loadMeshWasm } from "./loader.js";
import type { Scenario } from "./scenario.js";
import { scenarioFromQuery, scenarioJson } from "./scenario.js";

/** The page tick (§11.2 `setInterval(20 ms)`). */
export const TICK_MS = 20;
/** Longest simulated step per tick: a slow or throttled page lets time slip instead of
 *  freezing to catch up. */
export const MAX_STEP_MS = 100;

/** Timer functions (the global ones by default; fakes in tests). */
export interface Timers {
  setInterval(f: () => void, ms: number): unknown;
  clearInterval(handle: unknown): void;
}

export interface SimConvoyOptions extends LoadOptions {
  /** The scenario (merged over the query string's when both are given). */
  scenario?: Scenario;
  /** A query string to read the scenario from (e.g. `location.search`). */
  query?: string;
  /** An already loaded module instance to create the world in. */
  wasm?: MeshWasm;
  /** Page clock in ms (default `performance.now`). */
  now?: () => number;
  timers?: Timers;
  /** Tick period, ms (default 20). */
  tickMs?: number;
  /** Speech-like noise playback for remote talkers (see `TalkerNoisePlayer`). */
  audio?: TalkerNoisePlayer;
  /** Called when a tick throws (default: `console.error`); the driver keeps going. */
  onError?: (e: unknown) => void;
}

export type EventListener = (event: UiEvent) => void;

const defaultTimers: Timers = {
  setInterval: (f, ms) => globalThis.setInterval(f, ms),
  clearInterval: (h) => globalThis.clearInterval(h as ReturnType<typeof globalThis.setInterval>),
};

function defaultNow(): number {
  return globalThis.performance?.now() ?? Date.now();
}

/** A simulated convoy running in mesh_wasm.wasm. */
export class SimConvoy {
  readonly scenario: Scenario;
  readonly #wasm: MeshWasm;
  #world: number;
  readonly #listeners = new Set<EventListener>();
  readonly #lab = new LabFileStore();
  readonly #now: () => number;
  readonly #timers: Timers;
  readonly #tickMs: number;
  readonly #audio: TalkerNoisePlayer | undefined;
  readonly #onError: (e: unknown) => void;
  #interval: unknown = undefined;
  #last = 0;
  #queue: UiEvent[] = [];
  #dispatching = false;
  #snapshot: Snapshot | undefined;

  constructor(wasm: MeshWasm, scenario: Scenario, opts: SimConvoyOptions = {}) {
    this.#wasm = wasm;
    this.scenario = { wallMs: Date.now(), ...scenario };
    this.#world = wasm.worldNew(scenarioJson(this.scenario));
    if (this.#world === 0) {
      throw new MeshWasmError(`scenario refused: ${scenarioJson(this.scenario)}`);
    }
    this.#now = opts.now ?? defaultNow;
    this.#timers = opts.timers ?? defaultTimers;
    this.#tickMs = opts.tickMs ?? TICK_MS;
    this.#audio = opts.audio;
    this.#onError = opts.onError ?? ((e) => console.error("engine-web tick failed", e));
  }

  /** `true` while the 20 ms driver runs. */
  get running(): boolean {
    return this.#interval !== undefined;
  }

  /** The local phone's latest snapshot. */
  get snapshot(): Snapshot | undefined {
    return this.#snapshot;
  }

  /** Runs one JSON command (a mesh-api UiCommand or a `sim.*` command); returns the
   *  result JSON. Events it caused are delivered before this returns. */
  command(json: string): string {
    const out = this.#wasm.command(this.#live(), json);
    if (out === undefined) throw new MeshWasmError("world is gone");
    this.pump();
    return out;
  }

  /** `command` with objects in and out. */
  run(cmd: { type: string; [key: string]: unknown }): CommandResult & Record<string, unknown> {
    return JSON.parse(this.command(JSON.stringify(cmd))) as CommandResult & Record<string, unknown>;
  }

  /** Subscribes to the local phone's UiEvents; returns the unsubscribe function. */
  onEvent(cb: EventListener): () => void {
    this.#listeners.add(cb);
    return () => {
      this.#listeners.delete(cb);
    };
  }

  /** Starts advancing simulated time with the page clock every 20 ms. */
  start(): void {
    if (this.#interval !== undefined) return;
    this.#live();
    this.#last = this.#now();
    this.#interval = this.#timers.setInterval(() => this.#tick(), this.#tickMs);
  }

  /** Stops the clock (the world and its state stay). */
  stop(): void {
    if (this.#interval === undefined) return;
    this.#timers.clearInterval(this.#interval);
    this.#interval = undefined;
    this.#audio?.silence();
  }

  /** Advances simulated time by `ms` in ticks and delivers the events (tests, tools). */
  step(ms: number): void {
    let left = Math.max(0, ms);
    while (left > 0) {
      const dt = Math.min(left, this.#tickMs);
      this.#wasm.step(this.#live(), dt * 1000);
      this.pump();
      left -= dt;
    }
  }

  /** Delivers pending events and collects new Lab lines. */
  pump(): void {
    const world = this.#live();
    const lab = this.#wasm.pollLab(world);
    if (lab !== undefined && lab !== "") this.#lab.append(lab);
    const text = this.#wasm.pollEvents(world);
    if (text === undefined || text === "[]") return;
    for (const ev of JSON.parse(text) as UiEvent[]) this.#queue.push(ev);
    this.#drain();
  }

  /** Every Lab file the local phone wrote (labrun/1 JSONL and summary CSV per run). */
  labFiles(): LabFile[] {
    return this.#lab.files();
  }

  /** A Lab file by the path `labFinished.files` reported. */
  readFile(path: string): LabFile | undefined {
    return this.#lab.read(path);
  }

  /** Forgets a deleted run's files. */
  forgetRun(runId: string): void {
    this.#lab.delete(runId);
  }

  /** Stops the clock and frees the world. */
  dispose(): void {
    this.stop();
    if (this.#world !== 0) this.#wasm.worldFree(this.#world);
    this.#world = 0;
    this.#listeners.clear();
  }

  #live(): number {
    if (this.#world === 0) throw new MeshWasmError("convoy disposed");
    return this.#world;
  }

  #tick(): void {
    try {
      const now = this.#now();
      const dt = Math.min(Math.max(0, now - this.#last), MAX_STEP_MS);
      this.#last = now;
      if (dt <= 0 || this.#world === 0) return;
      this.#wasm.step(this.#world, dt * 1000);
      this.pump();
    } catch (e) {
      this.#onError(e);
    }
  }

  #drain(): void {
    if (this.#dispatching) return;
    this.#dispatching = true;
    try {
      for (let ev = this.#queue.shift(); ev !== undefined; ev = this.#queue.shift()) {
        this.#observe(ev);
        for (const cb of [...this.#listeners]) {
          try {
            cb(ev);
          } catch (e) {
            // A failing listener must not starve the others or stop the clock.
            this.#onError(e);
          }
        }
      }
    } finally {
      this.#dispatching = false;
    }
  }

  #observe(ev: UiEvent): void {
    if (ev.type === "snapshot") {
      this.#snapshot = ev.s;
      this.#audio?.update(ev.s);
    } else if (ev.type === "labFinished" && !this.#lab.hasCsv(ev.runId)) {
      const r = JSON.parse(
        this.#wasm.command(this.#live(), JSON.stringify({ type: "sim.labCsv", runId: ev.runId })) ??
          "{}",
      ) as { ok?: boolean; csv?: string };
      if (r.ok === true && typeof r.csv === "string") this.#lab.setCsv(ev.runId, r.csv);
    }
  }
}

/**
 * Loads mesh_wasm.wasm (unless `opts.wasm` is given) and creates a convoy. The scenario
 * is `opts.scenario` over the one read from `opts.query` (e.g. `?sim=chain5&loss=0.1`).
 */
export async function createSimConvoy(opts: SimConvoyOptions = {}): Promise<SimConvoy> {
  const wasm = opts.wasm ?? (await loadMeshWasm(opts));
  const fromQuery = opts.query !== undefined ? scenarioFromQuery(opts.query).scenario : {};
  return new SimConvoy(wasm, { ...fromQuery, ...opts.scenario }, opts);
}
