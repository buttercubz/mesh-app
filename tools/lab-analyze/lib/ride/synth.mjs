// Synthetic 3-phone ride (Tab, Galaxy, iPhone) for the self-test and the committed
// fixtures: idle triangle with control traffic only, wall clocks skewed by up to ±3 s,
// the iPhone lost at t = 60 s (scenario A4), Galaxy's "mic paused" with the permission
// granted (A1), crackle on the Tab (A2), a user mark at t = 70 s.

export const TRUE_START = Date.UTC(2026, 9, 22, 20, 4, 0);
export const RIDE_KEY = "9f3a71c2";
export const PHONES = [
  { node: 101, sid: "d-20261022-2004-tab1", platform: "android", model: "SM-X520", os: "Android 16 (API 36)", skewMs: 0, delayMs: 0, mono: 5_000_000_000 },
  { node: 202, sid: "d-20261022-2004-gal2", platform: "android", model: "SM-S921B", os: "Android 16 (API 36)", skewMs: 3000, delayMs: 400, mono: 7_000_000_000 },
  { node: 303, sid: "d-20261022-2004-iph3", platform: "ios", model: "iPhone15,5", os: "iOS 26.1", skewMs: -2500, delayMs: 900, mono: 1_000_000_000 },
];
const LINKS = [
  [101, 202, 1],
  [101, 303, 2],
  [202, 303, 3],
];
const LOST_AT_S = 60;
const DURATION_S = 120;

const iso = (ms) => new Date(ms).toISOString();
const r3 = (v) => Math.round(v * 1000) / 1000;

function dir(frames, voice = 0) {
  const byType = { nbr: 1, lstat: 1 };
  const byTypeB = { nbr: 33, lstat: 36 };
  if (frames > 2) {
    byType.ping = 1;
    byTypeB.ping = 32;
  }
  if (voice) {
    byType.voice = voice;
    byTypeB.voice = voice * 96;
  }
  const bytes = Object.values(byTypeB).reduce((a, b) => a + b, 0);
  return { frames: Object.values(byType).reduce((a, b) => a + b, 0), bytes, byType, byTypeB };
}

/** The lines of one phone (ts on its own clock). */
export function phoneLines(p, { rideKey = RIDE_KEY } = {}) {
  const ts = (tTrueS) => r3(tTrueS - p.delayMs / 1000);
  const lines = [];
  lines.push({
    t: "hdr", v: 1, schema: "ridelog/1", seg: 1, sessionId: p.sid, rideKey: null,
    startedAt: iso(TRUE_START + p.delayMs + p.skewMs), monoUs: p.mono,
    device: { platform: p.platform, model: p.model, os: p.os, app: "0.1.0 (12)", build: "debug", engine: "ffi" },
    node: null, role: null, caps: { l2cap: true }, settings: { ptt: "vox" }, privacy: { gps: "off", names: "none" }, verbose: false,
  });
  const e = (t, src, ev, fields = {}) => lines.push({ t: "e", ts: ts(t), src, ev, ...fields });
  e(1, "core", "session", { rideKey, node: p.node, role: p.node === 101 ? "leader" : "member" });
  if (p.node === 202) {
    e(2, "plat", "perm", { name: "microphone", state: "granted", via: "check", api: "checkSelfPermission" });
    e(3, "ui", "cmd", { name: "goLive", gesture: true, ok: true, ms: 41 });
    e(3.2, "plat", "fgs", { state: "started", types: "connectedDevice,microphone" });
    e(3.4, "plat", "audioOpen", { sr: 48000, ch: 1, cbFrames: 192, api: "oboe-aaudio" });
    e(3.5, "core", "micState", { state: "paused", cause: "builtInMicOff" });
  }
  if (p.node === 303) e(55, "plat", "app", { state: "background", locked: true });
  for (const [a, b, id] of LINKS) {
    if (a !== p.node && b !== p.node) continue;
    const peer = a === p.node ? b : a;
    e(1.5, "core", "linkUp", { link: id, kind: "ble-l2cap", role: a === p.node ? "central" : "peripheral", mtu: 247, psm: 129 });
    for (const t of [10, 40, 70, 100]) {
      if (peer === 303 || p.node === 303) {
        if (t >= LOST_AT_S) continue;
      }
      const peerPhone = PHONES.find((x) => x.node === peer);
      const offsetUs = peerPhone.mono - peerPhone.delayMs * 1000 - (p.mono - p.delayMs * 1000);
      e(t, "core", "clkOffset", { link: id, peer, offsetUs, rttUs: 38000 });
    }
    if (peer === 303 || p.node === 303) {
      e(LOST_AT_S, "core", "linkDown", { link: id, peer, reason: "supervisionTimeout", upS: 58, txB: 12000, rxB: 11000 });
    }
  }
  if (p.node === 303) e(LOST_AT_S + 0.5, "core", "partition", { islands: 2, mine: 1, others: [2] });
  else {
    e(LOST_AT_S + 0.5, "core", "partition", { islands: 2, mine: 2, others: [1] });
    e(LOST_AT_S + 1, "core", "heard", { peer: 303, lastHeardS: 1, lost: true });
  }
  if (p.node === 101) {
    e(30.2, "rt", "glitch", { kind: "underrun", n: 3, ms: 10 });
    lines.push({ t: "mark", ts: ts(70), src: "ui", label: "dropped" });
  }
  for (let t = 2; t <= DURATION_S; t++) {
    const links = [];
    for (const [a, b, id] of LINKS) {
      if (a !== p.node && b !== p.node) continue;
      const peer = a === p.node ? b : a;
      if ((peer === 303 || p.node === 303) && t > LOST_AT_S) continue;
      links.push({
        id, peer, kind: "ble-l2cap", role: a === p.node ? "central" : "peripheral", state: "up", rssi: -61, mtu: 247,
        rttP50: 38, rttP95: 61.5, loss: 0, grantKbps: 48, q: { voice: 0, ctrl: 0, bulk: 0, units: 0 }, wb: 0,
        tx: dir(t % 2 ? 3 : 2), rx: { ...dir(t % 2 ? 3 : 2), bad: 0 },
      });
    }
    const crackle = p.node === 101 && t >= 30 && t < 33;
    lines.push({
      t: "s", ts: ts(t), links,
      mesh: { nbrs: links.length, reach: links.length + 1, islands: t > LOST_AT_S ? 2 : 1, fwd: 0, dup: 1, suppressed: 0, tooOld: 0, pruned: 0, floorRefused: 0, voiceDropped: 0, burstsShed: 0, ownRefused: 0 },
      floor: { talkers: 0, me: "idle" },
      audio: {
        running: true, route: "builtIn", ladder: "L0", capFps: 100, renFps: 100,
        cbCapFrames: [192, 192], cbRenFrames: crackle ? [192, 480] : [192, 192], cbRenMax: 480, renTarget: 960,
        over: 0, under: crackle ? 2 : 0, overSamples: 0, underSamples: crackle ? 192 : 0,
        capDbfs: -50, renDbfs: -60, vad: false, ptt: false, muted: false, txFrames: 0,
      },
      jb: [],
      jbTotal: { recv: 50, late: crackle ? 3 : 0, dup: 0, fec: 0, plc: crackle ? 2 : 0, expanded: 0, dropped: 0 },
      dev: { bat: 0.81, chg: false, therm: "nominal", app: p.node === 303 && t >= 55 ? "background" : "foreground", locked: false, call: false },
      anchor: { session: true, location: false }, bt: "on", aware: "unsupported", distM: null, cpuMs: { engine: 18.4 },
      fx: { droppedDiag: 0, suppressed: 0, redacted: 0, droppedLines: 0 },
    });
  }
  lines.push({ t: "end", ts: ts(DURATION_S + 0.5), reason: "sessionEnd", lines: lines.length + 1, suppressed: 0, redacted: 0 });
  // Lines are written in time order by the recorder.
  const head = lines.shift();
  lines.sort((x, y) => x.ts - y.ts);
  return [head, ...lines];
}

/** JSONL text of one phone. */
export const phoneText = (p, o) => `${phoneLines(p, o).map((l) => JSON.stringify(l)).join("\n")}\n`;
