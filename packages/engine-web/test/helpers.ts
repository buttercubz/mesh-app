// Shared test helpers: one compiled module for the whole run, event recording with the
// engine-api contract guards, and small polling helpers over simulated time.
import type { Snapshot, UiEvent } from "@meshapp/engine-api";
import { explainUiEvent } from "@meshapp/engine-api";

import type { Scenario, SimConvoy, SimConvoyOptions } from "../src/index.js";
import { compileMeshWasm, createSimConvoy } from "../src/index.js";

let module: Promise<WebAssembly.Module> | undefined;

/** The package's mesh_wasm.wasm, compiled once per test file. */
export function wasmModule(): Promise<WebAssembly.Module> {
  module ??= compileMeshWasm();
  return module;
}

/** A line local(0) - bot1 - bot2 with scripted bots: every link carries both bots'
 *  voice, so the floor cap (2 per BLE link, §6.1) decides who may talk. */
export const LINE3: Scenario = { preset: "pair", nodes: 3, shadow: [[0, 2]], botsTalk: false, seed: 7 };

/** A convoy and every event it delivered, each checked against contract C1. */
export interface Recorded {
  c: SimConvoy;
  events: UiEvent[];
}

export async function convoy(opts: SimConvoyOptions = {}): Promise<Recorded> {
  const c = await createSimConvoy({ module: await wasmModule(), ...opts });
  const events: UiEvent[] = [];
  c.onEvent((e) => {
    const why = explainUiEvent(e);
    if (why !== undefined) throw new Error(`event breaks contract C1: ${why}: ${JSON.stringify(e)}`);
    events.push(e);
  });
  return { c, events };
}

/** Steps 20 ms ticks until `pred(snapshot)` holds or `limitMs` of simulated time passed. */
export function runUntil(c: SimConvoy, limitMs: number, pred: (s: Snapshot) => boolean): boolean {
  for (let t = 0; t <= limitMs; t += 20) {
    if (c.snapshot !== undefined && pred(c.snapshot)) return true;
    c.step(20);
  }
  return c.snapshot !== undefined && pred(c.snapshot);
}

/** Node ids of the current talkers, sorted. */
export function talkers(c: SimConvoy): number[] {
  return (c.snapshot?.talkers ?? []).map((t) => t.nodeId).sort((a, b) => a - b);
}

/** Runs a command that must succeed and returns its result. */
export function ok(c: SimConvoy, cmd: { type: string; [k: string]: unknown }): Record<string, unknown> {
  const r = c.run(cmd);
  if (!r.ok) throw new Error(`${JSON.stringify(cmd)} refused: ${JSON.stringify(r)}`);
  return r;
}

export interface BotInfo {
  index: number;
  name: string;
  nodeId: number | null;
  live: boolean;
  talking: boolean;
  away: boolean;
}

export function bots(c: SimConvoy): BotInfo[] {
  return ok(c, { type: "sim.info" })["bots"] as BotInfo[];
}
