// Groups sessions by ride and aligns their clocks.
//
// Every session maps its `ts` to wall time with `startedAt + ts`. Phone wall clocks
// can be seconds apart, so when the core recorded `clkOffset` events (PING/PONG
// min-RTT estimates; `offsetUs` = peer's engine clock − own engine clock) the wall
// mapping of each device is corrected relative to the lowest node: for one instant,
// mono_B = mono_A + offset, hence the wall error of B against A is
// (startB − monoB0) − (startA − monoA0) + offset (ms). Devices without offsets keep
// their own wall clock (method "wall").

const median = (xs) => {
  if (xs.length === 0) return null;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};

/** Groups sessions by rideKey (sessions without one share the `unknown` ride). */
export function groupRides(sessions, { ride = null } = {}) {
  const rides = new Map();
  for (const s of sessions) {
    const key = s.rideKey ?? "unknown";
    if (ride && key !== ride) continue;
    if (!rides.has(key)) rides.set(key, { rideKey: key, sessions: [] });
    rides.get(key).sessions.push(s);
  }
  return [...rides.values()];
}

/** Wall time (ms since epoch) of a line of session `s`. */
export const wallOf = (s, ts) => s.startedMs + ts * 1000 + (s.corrMs ?? 0);

/**
 * Sets `corrMs`, `alignMethod` and `alignResidualMs` on every session of `ride` and
 * returns a summary per device.
 */
export function alignRide(ride) {
  const byNode = new Map();
  for (const s of ride.sessions) {
    s.corrMs = 0;
    s.alignMethod = "wall";
    s.alignResidualMs = null;
    if (s.node !== null && !byNode.has(s.node)) byNode.set(s.node, s);
  }
  // Pairwise wall errors (ms) observed by both ends: err[a][b] = wall error of b vs a.
  const obs = new Map();
  const put = (a, b, v) => {
    const k = `${a}>${b}`;
    if (!obs.has(k)) obs.set(k, []);
    obs.get(k).push(v);
  };
  for (const s of byNode.values()) {
    for (const e of s.events) {
      if (e.ev !== "clkOffset" || typeof e.offsetUs !== "number") continue;
      const p = byNode.get(e.peer);
      if (!p || p === s) continue;
      const err = p.startedMs - p.monoUs / 1000 - (s.startedMs - s.monoUs / 1000) + e.offsetUs / 1000;
      put(s.node, p.node, err);
      put(p.node, s.node, -err);
    }
  }
  const nodes = [...byNode.keys()].sort((a, b) => a - b);
  if (nodes.length > 0) {
    const ref = byNode.get(nodes[0]);
    ref.alignMethod = "reference";
    const done = new Set([nodes[0]]);
    const queue = [nodes[0]];
    while (queue.length > 0) {
      const a = queue.shift();
      for (const b of nodes) {
        if (done.has(b)) continue;
        const xs = obs.get(`${a}>${b}`);
        if (!xs) continue;
        const m = median(xs);
        const sa = byNode.get(a);
        const sb = byNode.get(b);
        sb.corrMs = sa.corrMs - m;
        sb.alignMethod = `clkOffset via n${a}`;
        sb.alignResidualMs = median(xs.map((x) => Math.abs(x - m)));
        done.add(b);
        queue.push(b);
      }
    }
  }
  return ride.sessions.map((s) => ({
    device: s.label,
    sessionId: s.sessionId,
    model: s.device.model,
    platform: s.device.platform,
    method: s.alignMethod,
    corrMs: Math.round(s.corrMs),
    residualMs: s.alignResidualMs === null ? null : Math.round(s.alignResidualMs * 10) / 10,
  }));
}

/** Formats a wall time as `HH:MM:SS.s` (UTC). */
export function clock(ms) {
  const d = new Date(ms);
  const p = (n, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}.${Math.floor(d.getUTCMilliseconds() / 100)}`;
}

/** Minute bucket (`HH:MM`, UTC) of a wall time. */
export function minuteOf(ms) {
  return clock(ms).slice(0, 5);
}
