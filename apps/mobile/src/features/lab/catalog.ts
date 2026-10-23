// Field test bench catalog (WP-21, DESIGN §12.2, §12.5): the benches the Field test lists,
// their phase, phone count, default audio path and keep-alive anchor, the anchors their
// variants may use, the variant axes and the default traffic. It mirrors the WP-05
// catalog in crates/mesh-lab/src/catalog.rs (the engine applies the same defaults to any
// LabParams field left out); catalog.test.ts parses that file and fails on drift.
import type { LabAnchor, LabAudio, LabBench, LabPattern } from "@meshapp/engine-api";

/** Default traffic of a bench (`pattern: null` = no LPROBE traffic, the engine's "idle"). */
export interface BenchDefaults {
  readonly durationS: number;
  readonly pattern: LabPattern | null;
  readonly streams: number;
  readonly payloadB: number;
  readonly intervalMs: number;
  readonly pingHz: number;
}

/** One variant axis (each combination of values is a separate run). */
export interface BenchAxis {
  readonly key: string;
  readonly values: readonly string[];
}

export interface BenchSpec {
  readonly bench: LabBench;
  /** First phase in which the bench runs (DESIGN §14). */
  readonly phase: 0 | 1;
  /** Phones needed (min, max). */
  readonly devices: readonly [number, number];
  /** Default audio path (the headset-off variant for T0/T0b). */
  readonly audio: LabAudio;
  /** Default keep-alive anchor (never `none`). */
  readonly anchor: LabAnchor;
  /** Anchors the variants may use, default first. */
  readonly anchors: readonly LabAnchor[];
  readonly defaults: BenchDefaults;
  readonly axes: readonly BenchAxis[];
  /** Repetitions per variant (T3 trials, T5a click trains, T8 presses). */
  readonly repetitions: number;
}

const VOICE2: BenchDefaults = {
  durationS: 60,
  pattern: "voiceLike",
  streams: 2,
  payloadB: 118,
  intervalMs: 40,
  pingHz: 10,
};

const T0_PATTERNS = ["saturate", "voice1", "voice2", "voice3", "voice4", "ping", "pingUnderLoad"] as const;
const TRANSPORTS = ["l2cap", "gattNotify", "gattWrite"] as const;
const ROLES = ["central", "peripheral"] as const;
const HEADSET = ["off", "hfp"] as const;

/** Benches of the Field test, in list order: Phase 0 first, then the Phase 1 additions. */
export const BENCH_CATALOG: readonly BenchSpec[] = [
  {
    bench: "T0",
    phase: 0,
    devices: [2, 2],
    audio: "off",
    anchor: "audio",
    anchors: ["audio"],
    defaults: VOICE2,
    axes: [
      { key: "transport", values: TRANSPORTS },
      { key: "role", values: ROLES },
      { key: "headset", values: HEADSET },
      { key: "pattern", values: T0_PATTERNS },
      { key: "distance", values: ["near", "far"] },
    ],
    repetitions: 1,
  },
  {
    bench: "T0b",
    phase: 0,
    devices: [3, 3],
    audio: "off",
    anchor: "audio",
    anchors: ["audio"],
    defaults: VOICE2,
    axes: [
      { key: "headset", values: HEADSET },
      { key: "pattern", values: ["voice1", "voice2"] },
    ],
    repetitions: 1,
  },
  {
    bench: "T0c",
    phase: 0,
    devices: [3, 5],
    audio: "full",
    anchor: "audio",
    anchors: ["audio"],
    defaults: VOICE2,
    axes: [
      { key: "links", values: ["2", "3", "4"] },
      { key: "pacer", values: ["free", "capped"] },
    ],
    repetitions: 1,
  },
  {
    bench: "T1",
    phase: 0,
    devices: [2, 3],
    audio: "renderOnly",
    anchor: "audio",
    anchors: ["audio", "audioLocation"],
    defaults: { durationS: 1800, pattern: "pingUnderLoad", streams: 2, payloadB: 118, intervalMs: 40, pingHz: 5 },
    axes: [
      { key: "mount", values: ["mounted", "pocket", "tankBag"] },
      { key: "anchor", values: ["audio", "audioLocation"] },
    ],
    repetitions: 1,
  },
  {
    bench: "T2",
    phase: 0,
    devices: [1, 2],
    audio: "full",
    anchor: "audio",
    anchors: ["audio", "audioLocation", "none"],
    defaults: { durationS: 14_400, pattern: "voiceLike", streams: 1, payloadB: 118, intervalMs: 40, pingHz: 1 },
    axes: [
      { key: "anchor", values: ["audio", "audioLocation", "none"] },
      { key: "session", values: ["nonMixable", "mixable"] },
    ],
    repetitions: 1,
  },
  {
    bench: "T3",
    phase: 0,
    devices: [2, 3],
    audio: "renderOnly",
    anchor: "audio",
    anchors: ["audio", "none"],
    defaults: { durationS: 300, pattern: "ping", streams: 0, payloadB: 64, intervalMs: 40, pingHz: 5 },
    axes: [
      {
        key: "state",
        values: ["bothFg", "oneBg", "bothBgScreenOff", "bothBgLiveActivity", "pendingConnect", "behindEpoch"],
      },
      { key: "break", values: ["blacklist30s", "walkApart"] },
      { key: "anchor", values: ["audio", "none"] },
    ],
    repetitions: 5,
  },
  {
    bench: "T5a",
    phase: 0,
    devices: [1, 1],
    audio: "full",
    anchor: "audio",
    anchors: ["audio"],
    defaults: { durationS: 180, pattern: null, streams: 0, payloadB: 0, intervalMs: 2000, pingHz: 0 },
    axes: [{ key: "hfp", values: ["wideband", "narrowband"] }],
    repetitions: 50,
  },
  {
    bench: "T6",
    phase: 0,
    devices: [1, 2],
    audio: "full",
    anchor: "audio",
    anchors: ["audio", "audioLocation"],
    defaults: { durationS: 3600, pattern: "voiceLike", streams: 2, payloadB: 118, intervalMs: 40, pingHz: 1 },
    axes: [
      { key: "screen", values: ["off", "offNav", "onMount"] },
      { key: "charging", values: ["no", "yes"] },
      { key: "anchor", values: ["audio", "audioLocation"] },
    ],
    repetitions: 1,
  },
  {
    bench: "T5b",
    phase: 1,
    devices: [1, 2],
    audio: "full",
    anchor: "audio",
    anchors: ["audio"],
    defaults: { durationS: 600, pattern: null, streams: 0, payloadB: 0, intervalMs: 2000, pingHz: 0 },
    axes: [{ key: "check", values: ["echoReturn", "ducking", "buttons", "helmetIntercom"] }],
    repetitions: 20,
  },
  {
    bench: "T8",
    phase: 1,
    devices: [1, 1],
    audio: "full",
    anchor: "audio",
    anchors: ["audio"],
    defaults: { durationS: 300, pattern: null, streams: 0, payloadB: 0, intervalMs: 40, pingHz: 0 },
    axes: [],
    repetitions: 100,
  },
  {
    bench: "voxCal",
    phase: 1,
    devices: [1, 1],
    audio: "full",
    anchor: "audio",
    anchors: ["audio"],
    defaults: { durationS: 1800, pattern: null, streams: 0, payloadB: 0, intervalMs: 40, pingHz: 0 },
    axes: [
      { key: "sensitivity", values: ["low", "medium", "high"] },
      { key: "wind", values: ["calm", "windy"] },
    ],
    repetitions: 1,
  },
];

/** The listed benches, in list order. */
export const FIELD_TEST_BENCHES: readonly LabBench[] = BENCH_CATALOG.map((b) => b.bench);

/** Catalog entry of `bench`, or `undefined` for a bench the Field test does not list. */
export function benchSpec(bench: LabBench): BenchSpec | undefined {
  return BENCH_CATALOG.find((b) => b.bench === bench);
}

/** Audio path of a variant: `full` when a headset (HFP) is active, else the bench default. */
export function audioFor(spec: BenchSpec, headsetOn: boolean): LabAudio {
  return headsetOn ? "full" : spec.audio;
}

/** Traffic of a T0-style `pattern` axis value: pattern, streams (0 = none) and duration. */
export function patternAxis(value: string): { pattern: LabPattern; streams: number; durationS: number } | undefined {
  switch (value) {
    case "saturate":
      return { pattern: "saturate", streams: 0, durationS: 20 };
    case "voice1":
    case "voice2":
    case "voice3":
    case "voice4":
      return { pattern: "voiceLike", streams: Number(value.slice(5)), durationS: 60 };
    case "ping":
      return { pattern: "ping", streams: 0, durationS: 60 };
    case "pingUnderLoad":
      return { pattern: "pingUnderLoad", streams: 2, durationS: 60 };
    default:
      return undefined;
  }
}

/**
 * Axes that map onto typed LabParams fields; every other axis value is recorded in the
 * run's free `variant` label (DESIGN §12.3 `run` line).
 */
export const PARAM_AXES: ReadonlySet<string> = new Set(["transport", "role", "headset", "pattern", "anchor", "session"]);
