// The scenario a simulated convoy is built from (`mw_world_new`'s JSON, DESIGN §11.2
// `{preset, nodes, loss, seed}` plus the simulator's own knobs), and its query-parameter
// form for the web build: `?sim=chain5&loss=0.1&seed=7&roam=1`.

/** Geometry presets of mesh-sim (§13.2). */
export const PRESETS = ["pair", "chain5", "convoy15", "cluster15"] as const;
export type Preset = (typeof PRESETS)[number];

/** Every field is optional; the core fills the defaults (convoy15, 7 phones). */
export interface Scenario {
  /** Road geometry. */
  preset?: Preset;
  /** Phones in the world, the local one included (2..15). */
  nodes?: number;
  /** Extra per-transmission loss in [0, 1]. */
  loss?: number;
  /** World seed: the same scenario and commands give the same events. */
  seed?: number;
  /** Wall clock at creation (ms since the epoch), for Lab run ids. */
  wallMs?: number;
  /** Bots talk on their own random schedule (default true). */
  botsTalk?: boolean;
  /** Mean pause between two bursts of one bot, seconds (1..3600). */
  botTalkMeanS?: number;
  /** Pairs of world indices that never hear each other (forces a topology). */
  shadow?: Array<[number, number]>;
  /** Bots drop out of range and come back on their own schedule. */
  roam?: boolean;
  /** Mean time a roaming bot stays in range, seconds (5..3600). */
  roamMeanS?: number;
}

/** Page-side options read from the same query string. */
export interface PageOptions {
  /** Play speech-like noise while somebody else talks (`?audio=1`). */
  audio: boolean;
}

function num(v: string | null): number | undefined {
  if (v === null || v.trim() === "") return undefined;
  const x = Number(v);
  return Number.isFinite(x) ? x : undefined;
}

function bool(v: string | null): boolean | undefined {
  if (v === null) return undefined;
  if (v === "" || v === "1" || v === "true" || v === "on") return true;
  if (v === "0" || v === "false" || v === "off") return false;
  return undefined;
}

/** `0-2,1-3` → `[[0,2],[1,3]]`; malformed pairs are skipped. */
function pairs(v: string | null): Array<[number, number]> | undefined {
  if (v === null || v.trim() === "") return undefined;
  const out: Array<[number, number]> = [];
  for (const part of v.split(",")) {
    const m = /^\s*(\d+)\s*[-:]\s*(\d+)\s*$/.exec(part);
    if (m !== null) out.push([Number(m[1]), Number(m[2])]);
  }
  return out.length > 0 ? out : undefined;
}

/**
 * Reads the scenario from a query string (`location.search`). Parameters the
 * simulator does not know are ignored (the app has its own), unknown presets are
 * ignored too so a typo falls back to the default world instead of a blank page.
 */
export function scenarioFromQuery(search: string): { scenario: Scenario; page: PageOptions } {
  const q = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
  const scenario: Scenario = {};
  const preset = q.get("sim") ?? q.get("preset");
  if (preset !== null && (PRESETS as readonly string[]).includes(preset)) {
    scenario.preset = preset as Preset;
  }
  const nodes = num(q.get("nodes"));
  if (nodes !== undefined) scenario.nodes = Math.round(nodes);
  const loss = num(q.get("loss"));
  if (loss !== undefined) scenario.loss = loss;
  const seed = num(q.get("seed"));
  if (seed !== undefined && seed >= 0) scenario.seed = Math.floor(seed);
  const botsTalk = bool(q.get("botsTalk"));
  if (botsTalk !== undefined) scenario.botsTalk = botsTalk;
  const talkMean = num(q.get("talkMeanS") ?? q.get("botTalkMeanS"));
  if (talkMean !== undefined) scenario.botTalkMeanS = talkMean;
  const shadow = pairs(q.get("shadow"));
  if (shadow !== undefined) scenario.shadow = shadow;
  const roam = bool(q.get("roam"));
  if (roam !== undefined) scenario.roam = roam;
  const roamMean = num(q.get("roamMeanS"));
  if (roamMean !== undefined) scenario.roamMeanS = roamMean;
  return { scenario, page: { audio: bool(q.get("audio")) ?? false } };
}

/** The JSON `mw_world_new` takes (undefined fields dropped). */
export function scenarioJson(s: Scenario): string {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(s)) {
    if (v !== undefined) out[k] = v;
  }
  return JSON.stringify(out);
}
