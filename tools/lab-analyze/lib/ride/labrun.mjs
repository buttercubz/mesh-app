// `--as-labrun`: projects a ridelog/1 session onto labrun/1 lines (bench "ride") so the
// T1/T2 analysis of the Lab tooling can run on real rides (DESIGN §12.1 "Ride
// recorder"). Only what both schemas share is kept; the projection is validated
// against labrun/1 by the self-test.

const THERM = new Set(["nominal", "fair", "serious", "critical", "none", "light", "moderate", "severe", "emergency", "shutdown", "unknown"]);
const LADDER = new Set(["L0", "L1", "L2", "L3"]);
const BT = new Set(["on", "off", "unauthorized", "resetting", "unsupported", "unknown"]);

/** labrun/1 `runId` for a session (`r-YYYYMMDD-HHMM-xxxx`). */
export function runIdOf(s) {
  const m = /^d-(\d{8})-(\d{4})-([0-9a-z]{4})$/.exec(s.sessionId);
  return m ? `r-${m[1]}-${m[2]}-${m[3]}` : "r-19700101-0000-0000";
}

/** The labrun/1 lines of one session of `ride`. */
export function toLabrun(ride, s) {
  const nodes = ride.sessions.map((x) => x.node).filter((n) => n !== null);
  const lowest = nodes.length ? Math.min(...nodes) : null;
  const run = {
    t: "run",
    v: 1,
    runId: runIdOf(s),
    bench: "ride",
    variant: { rideKey: s.rideKey ?? "unknown" },
    audio: "full",
    anchor: "audio",
    startedAt: s.startedAt,
    device: { model: s.device.model, os: s.device.os, app: s.device.app, platform: s.device.platform },
    labNode: s.node ?? 0,
    role: s.node !== null && s.node === lowest ? "controller" : "follower",
    peers: ride.sessions
      .filter((x) => x !== s && x.node !== null)
      .map((x) => ({ labNode: x.node, model: x.device.model })),
    params: { durationS: Math.round(s.samples.at(-1)?.ts ?? 0), pattern: "idle", gps: false },
  };
  const lines = [run];
  for (const smp of s.samples) {
    const a = smp.audio;
    const dev = smp.dev ?? {};
    lines.push({
      t: "s",
      ts: smp.ts,
      links: smp.links.map((l) => ({
        id: l.id,
        kind: l.kind,
        role: l.role ?? null,
        peer: l.peer ?? null,
        rssi: l.rssi ?? null,
        txKbps: Math.round(((l.tx.bytes * 8) / 1000) * 10) / 10,
        rxKbps: Math.round(((l.rx.bytes * 8) / 1000) * 10) / 10,
        loss: typeof l.loss === "number" ? l.loss : 0,
        rttP50: l.rttP50 ?? null,
        rttP95: l.rttP95 ?? null,
        owdP95: null,
        stalls: 0,
        maxWrite: null,
      })),
      audio: {
        capFps: a.capFps ?? null,
        renFps: a.renFps ?? null,
        running: a.running,
        route: a.route || "unknown",
        ladder: LADDER.has(a.ladder) ? a.ladder : "L0",
        xruns: (a.under ?? 0) + (a.over ?? 0),
      },
      dev: {
        bat: dev.bat ?? null,
        chg: dev.chg ?? null,
        therm: THERM.has(dev.therm) ? dev.therm : "unknown",
        app: dev.app === "background" || dev.app === "inactive" ? dev.app : "foreground",
        locked: dev.locked ?? null,
      },
      anchor: {
        ok: smp.anchor?.session ?? null,
        session: smp.anchor?.session ?? null,
        location: smp.anchor?.location ?? null,
        liveActivity: null,
      },
      bt: BT.has(smp.bt) ? smp.bt : "unknown",
    });
  }
  return lines;
}
