// Report sections of `lab-analyze ride` (§4 of the diagnostics spec): timeline,
// per-link traffic, audio glitches, partitions and marks. Each section returns plain
// rows; `format.mjs` prints them and `--csv` writes them.
import { clock, minuteOf, wallOf } from "./align.mjs";

/** Events shown in the default (warn-level) timeline. */
const WARN = new Set([
  "linkDown",
  "connectFail",
  "abort",
  "partition",
  "captureLost",
  "micState",
  "route",
  "interruption",
  "glitch",
  "engineCrash",
  "engineStart",
  "engineStop",
  "wouldBlockBurst",
  "suppressed",
  "fgs",
  "app",
  "uiError",
  "gattError",
  "effectOverflow",
  "audioOpen",
  "audioClose",
]);

function isWarn(e) {
  if (WARN.has(e.ev)) return true;
  if (e.ev === "heard") return e.lost === true;
  if (e.ev === "handshake") return e.ok === false;
  if (e.ev === "perm") return e.state !== "granted";
  if (e.ev === "cmd") return e.ok === false || e.name === "goLive" || e.name === "resumeMic";
  if (e.ev === "radioState") return e.ble !== "on";
  return false;
}

const FIELDS_SKIP = new Set(["t", "ts", "src", "ev"]);

/** `k=v` text of an event's fields. */
export function fieldsText(e) {
  return Object.entries(e)
    .filter(([k]) => !FIELDS_SKIP.has(k))
    .map(([k, v]) => `${k}=${Array.isArray(v) ? `[${v.join(",")}]` : v}`)
    .join(" ");
}

/** Merged timeline rows `{ wall, time, device, src, what, text }`. */
export function timeline(ride, { all = false, from = null, to = null } = {}) {
  const rows = [];
  for (const s of ride.sessions) {
    for (const e of s.events) {
      if (!all && !isWarn(e)) continue;
      rows.push({ wall: wallOf(s, e.ts), device: s.label, src: e.src, what: e.ev, text: fieldsText(e) });
    }
    for (const m of s.marks) {
      rows.push({ wall: wallOf(s, m.ts), device: s.label, src: m.src ?? "ui", what: "MARK", text: m.label });
    }
  }
  return rows
    .filter((r) => (from === null || r.wall >= from) && (to === null || r.wall <= to))
    .sort((a, b) => a.wall - b.wall)
    .map((r) => ({ ...r, time: clock(r.wall) }));
}

const addMap = (into, from) => {
  for (const [k, v] of Object.entries(from ?? {})) into[k] = (into[k] ?? 0) + v;
};

/**
 * Per-minute, per-link traffic: both ends side by side (matched by node and peer).
 * Also returns the idle overhead per node and minute: all non-voice bytes/s sent.
 */
export function traffic(ride) {
  const cells = new Map();
  const overhead = new Map();
  for (const s of ride.sessions) {
    for (const smp of s.samples) {
      const minute = minuteOf(wallOf(s, smp.ts));
      const ok = `${minute}|${s.label}`;
      if (!overhead.has(ok)) overhead.set(ok, { minute, device: s.label, node: s.node, secs: 0, nonVoiceB: 0, voiceB: 0 });
      const o = overhead.get(ok);
      o.secs += 1;
      for (const l of smp.links) {
        const key = `${minute}|${s.label}|${l.id}`;
        if (!cells.has(key)) {
          cells.set(key, {
            minute,
            device: s.label,
            node: s.node,
            link: l.id,
            peer: l.peer ?? null,
            kind: l.kind,
            secs: 0,
            txB: 0,
            rxB: 0,
            txTypes: {},
            rxTypes: {},
            txTypesB: {},
            wb: 0,
            qMax: 0,
            loss: [],
            rttP95: null,
            grant: null,
          });
        }
        const c = cells.get(key);
        c.secs += 1;
        c.txB += l.tx.bytes;
        c.rxB += l.rx.bytes;
        addMap(c.txTypes, l.tx.byType);
        addMap(c.rxTypes, l.rx.byType);
        addMap(c.txTypesB, l.tx.byTypeB);
        c.wb += l.wb ?? 0;
        const q = l.q ?? {};
        c.qMax = Math.max(c.qMax, (q.voice ?? 0) + (q.ctrl ?? 0) + (q.bulk ?? 0));
        if (typeof l.loss === "number") c.loss.push(l.loss);
        if (typeof l.rttP95 === "number") c.rttP95 = Math.max(c.rttP95 ?? 0, l.rttP95);
        if (typeof l.grantKbps === "number") c.grant = l.grantKbps;
        const voiceB = l.tx.byTypeB?.voice ?? 0;
        o.voiceB += voiceB;
        o.nonVoiceB += l.tx.bytes - voiceB;
      }
    }
  }
  const rows = [...cells.values()].map((c) => ({
    minute: c.minute,
    device: c.device,
    node: c.node,
    link: c.link,
    peer: c.peer,
    kind: c.kind,
    txKbps: kbps(c.txB, c.secs),
    rxKbps: kbps(c.rxB, c.secs),
    txTypes: c.txTypes,
    rxTypes: c.rxTypes,
    wb: c.wb,
    qMax: c.qMax,
    loss: c.loss.length ? round(c.loss.reduce((a, b) => a + b, 0) / c.loss.length, 4) : null,
    rttP95: c.rttP95,
    grantKbps: c.grant,
  }));
  // Match the far end of each link: same minute, node ↔ peer swapped.
  for (const r of rows) {
    const far = rows.find((o) => o.minute === r.minute && o.node === r.peer && o.peer === r.node && r.node !== null);
    r.farTxKbps = far ? far.txKbps : null;
    r.farRxKbps = far ? far.rxKbps : null;
  }
  rows.sort((a, b) => a.minute.localeCompare(b.minute) || a.device.localeCompare(b.device) || a.link - b.link);
  const idle = [...overhead.values()]
    .map((o) => ({
      minute: o.minute,
      device: o.device,
      node: o.node,
      nonVoiceKbps: kbps(o.nonVoiceB, o.secs),
      voiceKbps: kbps(o.voiceB, o.secs),
    }))
    .sort((a, b) => a.minute.localeCompare(b.minute) || a.device.localeCompare(b.device));
  return { links: rows, idle };
}

const round = (v, d = 1) => Math.round(v * 10 ** d) / 10 ** d;
const kbps = (bytes, secs) => (secs > 0 ? round((bytes * 8) / 1000 / secs, 2) : 0);

/** Per-device audio summary plus the first permission/mic mismatch (scenario A1). */
export function audio(ride) {
  return ride.sessions.map((s) => {
    const perMinute = new Map();
    let recv = 0;
    let late = 0;
    let plc = 0;
    let fec = 0;
    let cbChanges = 0;
    let targetMin = null;
    let targetMax = null;
    for (const smp of s.samples) {
      const minute = minuteOf(wallOf(s, smp.ts));
      if (!perMinute.has(minute)) perMinute.set(minute, { minute, under: 0, over: 0 });
      const m = perMinute.get(minute);
      m.under += smp.audio.under ?? 0;
      m.over += smp.audio.over ?? 0;
      const cb = smp.audio.cbRenFrames;
      if (Array.isArray(cb) && cb[0] !== cb[1]) cbChanges += 1;
      const tot = smp.jbTotal;
      if (tot) {
        recv += tot.recv;
        late += tot.late;
        plc += tot.plc;
        fec += tot.fec ?? 0;
      }
      for (const j of smp.jb) {
        if (typeof j.targetMs === "number") {
          targetMin = targetMin === null ? j.targetMs : Math.min(targetMin, j.targetMs);
          targetMax = targetMax === null ? j.targetMs : Math.max(targetMax, j.targetMs);
        }
      }
    }
    const per1000 = (n) => (recv > 0 ? round((n * 1000) / recv, 1) : null);
    const ev = (names) => s.events.filter((e) => names.includes(e.ev));
    const transitions = ev(["captureLost", "micState", "captureStart", "captureStop"]).map(
      (e) => `${clock(wallOf(s, e.ts))} ${e.ev} ${fieldsText(e)}`,
    );
    const perms = ev(["perm"]).map((e) => `${e.name}=${e.state}${e.api ? ` (${e.api})` : ""}`);
    cbChanges += ev(["glitch"]).filter((e) => e.kind === "cbSizeChange").length;
    return {
      device: s.label,
      underPerMin: [...perMinute.values()],
      cbSizeChanges: cbChanges,
      latePer1000: per1000(late),
      plcPer1000: per1000(plc),
      fecPer1000: per1000(fec),
      jbTargetMs: targetMin === null ? null : [targetMin, targetMax],
      routeChanges: ev(["route"]).length,
      glitches: ev(["glitch"]).reduce((n, e) => n + (e.n ?? 1), 0),
      transitions,
      perms: [...new Set(perms)],
      mismatch: micMismatch(s),
    };
  });
}

/** Scenario A1: microphone permission granted but the banner says the mic is off. */
export function micMismatch(s) {
  let granted = null;
  for (const e of s.events) {
    if (e.ev === "perm" && e.name === "microphone") granted = e.state === "granted" ? e : null;
    if (granted && e.ev === "micState" && e.state !== "live") {
      return `perm granted (${granted.api ?? "?"}) but micState=${e.state} cause=${e.cause ?? "?"} at ${clock(wallOf(s, e.ts))}`;
    }
  }
  return null;
}

/** Islands over time and who hears whom per minute (peer links with rx > 0). */
export function partitions(ride) {
  const islands = [];
  const hears = new Map();
  for (const s of ride.sessions) {
    for (const e of s.events.filter((x) => x.ev === "partition")) {
      islands.push({ time: clock(wallOf(s, e.ts)), wall: wallOf(s, e.ts), device: s.label, islands: e.islands, mine: e.mine });
    }
    for (const smp of s.samples) {
      const minute = minuteOf(wallOf(s, smp.ts));
      const key = `${minute}|${s.label}`;
      if (!hears.has(key)) hears.set(key, { minute, device: s.label, heard: new Set(), islandsMax: 0 });
      const h = hears.get(key);
      h.islandsMax = Math.max(h.islandsMax, smp.mesh?.islands ?? 0);
      for (const l of smp.links) if (l.rx.frames > 0 && l.peer !== null && l.peer !== undefined) h.heard.add(l.peer);
    }
  }
  islands.sort((a, b) => a.wall - b.wall);
  const matrix = [...hears.values()]
    .map((h) => ({ minute: h.minute, device: h.device, heard: [...h.heard].sort((a, b) => a - b), islandsMax: h.islandsMax }))
    .sort((a, b) => a.minute.localeCompare(b.minute) || a.device.localeCompare(b.device));
  return { islands, matrix };
}

/** Every mark with ±`windowS` seconds of events from every device. */
export function marks(ride, windowS = 10) {
  const all = timeline(ride, { all: true });
  return all
    .filter((r) => r.what === "MARK")
    .map((m) => ({
      mark: m,
      context: all.filter((r) => r !== m && Math.abs(r.wall - m.wall) <= windowS * 1000),
    }));
}
