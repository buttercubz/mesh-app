// Synthetic per-second link statistics of the fake Lab peer. The numbers follow the
// shape of the research expectations (DESIGN §12.2: voice-like 118 B / 40 ms streams,
// tens of ms RTT on a 1-5 m BLE link, a few per mille loss) so the Field test live view,
// sparklines and verdict chips can be developed. Nothing here is a measurement.
import type { ResolvedLabParams } from "./lab-params.js";
import type { Rng } from "./rng.js";

/** One second of traffic on the (single) fake Lab link. */
export interface LinkSecond {
  txKbps: number;
  rxKbps: number;
  loss: number;
  rttP50: number;
  rttP95: number;
  owdP95: number;
  rssi: number;
  stalls: number;
  /** Individual ping RTTs of this second, ms (raw data for the summary percentiles). */
  pings: number[];
}

/** Offered load of the run's traffic pattern, kbit/s. */
export function offeredKbps(p: ResolvedLabParams): number {
  const voice = (p.streams * p.payloadB * 8) / p.intervalMs;
  const ping = (64 * 8 * p.pingHz) / 1000;
  switch (p.pattern) {
    case "saturate":
      return 160;
    case "voiceLike":
      return voice;
    case "ping":
      return ping;
    case "pingUnderLoad":
      return voice + ping;
  }
}

const round = (v: number, digits: number): number => {
  const f = 10 ** digits;
  return Math.round(v * f) / f;
};

/** Generates one second of link statistics for `p`. */
export function linkSecond(rng: Rng, p: ResolvedLabParams): LinkSecond {
  const offered = offeredKbps(p);
  const saturating = p.pattern === "saturate";
  const lossBase = saturating ? rng.range(0.008, 0.035) : rng.range(0.001, 0.012);
  const loss = Math.min(0.5, lossBase + (p.headset ? rng.range(0, 0.008) : 0));
  const transportExtra = p.transport === "l2cap" ? 0 : rng.range(4, 12);
  const loadExtra = saturating ? rng.range(15, 30) : 0;
  const pingCount = Math.max(1, Math.round(p.pingHz));
  const pings: number[] = [];
  for (let i = 0; i < pingCount; i++) {
    const jitter = rng.chance(0.1) ? rng.range(15, 35) : rng.range(0, 12);
    pings.push(round(28 + transportExtra + loadExtra + jitter, 3));
  }
  const sorted = [...pings].sort((a, b) => a - b);
  const rttP50 = percentile(sorted, 0.5);
  const rttP95 = percentile(sorted, 0.95);
  const tx = saturating ? rng.range(140, 175) : offered * (1 + rng.range(-0.02, 0.02));
  return {
    txKbps: round(tx, 1),
    rxKbps: round(tx * (1 - loss), 1),
    loss: round(loss, 4),
    rttP50: round(rttP50, 1),
    rttP95: round(rttP95, 1),
    owdP95: round(rttP95 / 2 + rng.range(-2, 2), 1),
    rssi: Math.round(rng.range(-68, -56)),
    stalls: rng.chance(0.02) ? 1 : 0,
    pings,
  };
}

/** Nearest-rank percentile of an ascending array (`q` in [0, 1]); 0 when empty. */
export function percentile(sorted: readonly number[], q: number): number {
  if (sorted.length === 0) return 0;
  const rank = Math.min(sorted.length - 1, Math.max(0, Math.ceil(q * sorted.length) - 1));
  return sorted[rank] ?? 0;
}

/** Fake GPS fix near the given origin (≈ 10 m walk), for runs with GPS consent. */
export function gpsNear(
  rng: Rng,
  origin: { lat: number; lon: number },
): { lat: number; lon: number; spd: number; acc: number } {
  return {
    lat: round(origin.lat + rng.range(-0.00005, 0.00005), 6),
    lon: round(origin.lon + rng.range(-0.00005, 0.00005), 6),
    spd: round(rng.range(0, 1.5), 1),
    acc: round(rng.range(3, 8), 1),
  };
}

/** Great-circle distance in metres. */
export function distanceM(a: { lat: number; lon: number }, b: { lat: number; lon: number }): number {
  const r = 6_371_000;
  const rad = Math.PI / 180;
  const dLat = (b.lat - a.lat) * rad;
  const dLon = (b.lon - a.lon) * rad;
  const h =
    Math.sin(dLat / 2) ** 2 + Math.cos(a.lat * rad) * Math.cos(b.lat * rad) * Math.sin(dLon / 2) ** 2;
  return round(2 * r * Math.asin(Math.min(1, Math.sqrt(h))), 1);
}
