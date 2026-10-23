// Bench catalog of the fake Lab peer: which benches Phase 0 runs (WP-21 bench list),
// their defaults (DESIGN §12.2) and `labStart` parameter range checks. The real
// catalog lives in mesh-lab (WP-05); these values only make the browser fake behave
// like a phone so the Field test UI can be built against it.
import type {
  LabAnchor,
  LabAudio,
  LabBench,
  LabLinkRole,
  LabParams,
  LabPattern,
  LabTransport,
} from "@meshapp/engine-api";

/** Benches the Phase 0 Field test runs; later-phase benches answer `notSupported`. */
export const PHASE0_BENCHES = [
  "T0",
  "T0b",
  "T0c",
  "T1",
  "T2",
  "T3",
  "T5a",
  "T6",
] as const satisfies readonly LabBench[];

interface BenchDefaults {
  durationS: number;
  pattern: LabPattern;
  streams: number;
  payloadB: number;
  intervalMs: number;
  pingHz: number;
}

const VOICE = { streams: 2, payloadB: 118, intervalMs: 40 } as const;

const BENCH_DEFAULTS: Record<(typeof PHASE0_BENCHES)[number], BenchDefaults> = {
  T0: { durationS: 60, pattern: "voiceLike", ...VOICE, pingHz: 10 },
  T0b: { durationS: 60, pattern: "voiceLike", ...VOICE, pingHz: 10 },
  T0c: { durationS: 60, pattern: "voiceLike", ...VOICE, pingHz: 10 },
  T1: { durationS: 1800, pattern: "pingUnderLoad", ...VOICE, pingHz: 5 },
  T2: { durationS: 14400, pattern: "voiceLike", ...VOICE, streams: 1, pingHz: 1 },
  T3: { durationS: 300, pattern: "ping", ...VOICE, payloadB: 64, pingHz: 5 },
  T5a: { durationS: 60, pattern: "ping", ...VOICE, payloadB: 64, pingHz: 10 },
  T6: { durationS: 3600, pattern: "voiceLike", ...VOICE, pingHz: 1 },
};

/** `labStart` parameters with every bench default applied. */
export interface ResolvedLabParams {
  bench: LabBench;
  audio: LabAudio;
  anchor: LabAnchor;
  gps: boolean;
  durationS: number;
  pattern: LabPattern;
  streams: number;
  payloadB: number;
  intervalMs: number;
  pingHz: number;
  transport: LabTransport;
  role: LabLinkRole;
  headset: boolean;
  mixable: boolean;
  variant: string | null;
}

export function isPhase0Bench(bench: LabBench): bench is (typeof PHASE0_BENCHES)[number] {
  return (PHASE0_BENCHES as readonly LabBench[]).includes(bench);
}

const isIntIn = (v: number, lo: number, hi: number): boolean =>
  Number.isInteger(v) && v >= lo && v <= hi;

/**
 * Range checks the type guards cannot express. Returns the name of the first bad field,
 * or `undefined` when the parameters are acceptable.
 */
export function invalidLabParam(params: LabParams): string | undefined {
  const checks: [string, number | null | undefined, (v: number) => boolean][] = [
    ["durationS", params.durationS, (v) => isIntIn(v, 1, 86_400)],
    ["streams", params.streams, (v) => isIntIn(v, 1, 4)],
    ["payloadB", params.payloadB, (v) => isIntIn(v, 16, 1024)],
    ["intervalMs", params.intervalMs, (v) => isIntIn(v, 5, 1000)],
    ["pingHz", params.pingHz, (v) => Number.isFinite(v) && v > 0 && v <= 50],
  ];
  for (const [name, value, ok] of checks) {
    if (value !== null && value !== undefined && !ok(value)) return name;
  }
  if (typeof params.variant === "string" && new TextEncoder().encode(params.variant).length > 64) {
    return "variant";
  }
  return undefined;
}

/** Applies the bench defaults to every field left out (or `null`). */
export function resolveLabParams(
  bench: (typeof PHASE0_BENCHES)[number],
  params: LabParams,
): ResolvedLabParams {
  const d = BENCH_DEFAULTS[bench];
  return {
    bench,
    audio: params.audio,
    anchor: params.anchor,
    gps: params.gps,
    durationS: params.durationS ?? d.durationS,
    pattern: params.pattern ?? d.pattern,
    streams: params.streams ?? d.streams,
    payloadB: params.payloadB ?? d.payloadB,
    intervalMs: params.intervalMs ?? d.intervalMs,
    pingHz: params.pingHz ?? d.pingHz,
    transport: params.transport ?? "l2cap",
    role: params.role ?? "central",
    headset: params.headset ?? params.audio === "full",
    mixable: params.mixable ?? false,
    variant: params.variant ?? null,
  };
}
