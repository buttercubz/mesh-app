// `lab-analyze ride`: merges ridelog/1 files of several phones by rideKey and prints
// the timeline, per-link traffic, audio and partition report (§4 of the diagnostics
// spec). Exit code 1 when any line fails the schema (nothing is silently skipped).
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { toCsv } from "../csv.mjs";
import { loadRideSchema } from "../schema.mjs";
import { alignRide, groupRides } from "./align.mjs";
import { formatRide } from "./format.mjs";
import { toLabrun } from "./labrun.mjs";
import { readRides } from "./read.mjs";
import { audio, marks, partitions, timeline, traffic } from "./sections.mjs";

/** `HH:MM` (UTC) on the day of `refMs` → ms, or null. */
export function parseHm(hm, refMs) {
  if (!hm) return null;
  const m = /^(\d{1,2}):(\d{2})$/.exec(hm);
  if (!m) throw new Error(`bad time ${hm} (expected HH:MM, UTC)`);
  const d = new Date(refMs);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), Number(m[1]), Number(m[2]));
}

/** Analyses already-read sessions. */
export function analyzeRides(sessions, opts = {}) {
  return groupRides(sessions, { ride: opts.ride ?? null }).map((ride) => {
    const alignment = alignRide(ride);
    const ref = ride.sessions[0]?.startedMs ?? 0;
    const from = parseHm(opts.from, ref);
    const to = parseHm(opts.to, ref);
    return {
      rideKey: ride.rideKey,
      ride,
      alignment,
      timelineAll: Boolean(opts.all),
      timeline: timeline(ride, { all: Boolean(opts.all), from, to }),
      traffic: traffic(ride),
      audio: audio(ride),
      partitions: partitions(ride),
      marks: marks(ride),
    };
  });
}

/** Writes the CSV tables of every ride into `dir`; returns the paths. */
export function writeRideCsv(results, dir) {
  mkdirSync(dir, { recursive: true });
  const written = [];
  for (const r of results) {
    const base = join(dir, `ride-${r.rideKey}`);
    const files = {
      [`${base}-timeline.csv`]: toCsv(
        ["time", "device", "src", "event", "fields"],
        r.timeline.map((t) => [t.time, t.device, t.src, t.what, t.text]),
      ),
      [`${base}-traffic.csv`]: toCsv(
        ["minute", "device", "node", "link", "peer", "kind", "txKbps", "rxKbps", "farTxKbps", "farRxKbps", "loss", "rttP95", "wb", "qMax", "grantKbps", "txTypes"],
        r.traffic.links.map((l) => [
          l.minute, l.device, l.node, l.link, l.peer, l.kind, l.txKbps, l.rxKbps, l.farTxKbps, l.farRxKbps,
          l.loss, l.rttP95, l.wb, l.qMax, l.grantKbps,
          Object.entries(l.txTypes).map(([k, v]) => `${k}:${v}`).join(" "),
        ]),
      ),
      [`${base}-idle.csv`]: toCsv(
        ["minute", "device", "node", "nonVoiceKbps", "voiceKbps"],
        r.traffic.idle.map((o) => [o.minute, o.device, o.node, o.nonVoiceKbps, o.voiceKbps]),
      ),
      [`${base}-audio.csv`]: toCsv(
        ["device", "cbSizeChanges", "latePer1000", "plcPer1000", "fecPer1000", "jbTargetMin", "jbTargetMax", "routeChanges", "glitches", "mismatch"],
        r.audio.map((a) => [
          a.device, a.cbSizeChanges, a.latePer1000, a.plcPer1000, a.fecPer1000,
          a.jbTargetMs?.[0], a.jbTargetMs?.[1], a.routeChanges, a.glitches, a.mismatch,
        ]),
      ),
    };
    for (const [path, text] of Object.entries(files)) {
      writeFileSync(path, text);
      written.push(path);
    }
  }
  return written;
}

/** Writes one labrun/1 file per session (`--as-labrun`); returns the paths. */
export function writeAsLabrun(results, dir) {
  mkdirSync(dir, { recursive: true });
  const written = [];
  for (const r of results) {
    for (const s of r.ride.sessions) {
      const lines = toLabrun(r.ride, s);
      const path = join(dir, `${lines[0].runId}-n${s.node ?? 0}.jsonl`);
      writeFileSync(path, `${lines.map((l) => JSON.stringify(l)).join("\n")}\n`);
      written.push(path);
    }
  }
  return written;
}

/** The `ride` command; returns the exit code. */
export function rideCommand(opts) {
  const { sessions, problems } = readRides(opts.inputs, loadRideSchema());
  for (const p of problems) console.error(`invalid: ${p}`);
  const results = analyzeRides(sessions, opts);
  if (opts.json) {
    const plain = results.map((r) => Object.fromEntries(Object.entries(r).filter(([k]) => k !== "ride")));
    console.log(JSON.stringify(plain, null, 2));
  } else {
    for (const r of results) console.log(formatRide(r));
  }
  if (opts.csv) for (const p of writeRideCsv(results, opts.csv)) console.log(`wrote ${p}`);
  if (opts.asLabrun) for (const p of writeAsLabrun(results, opts.asLabrun)) console.log(`wrote ${p}`);
  if (problems.length > 0) {
    console.error(`lab-analyze ride: ${problems.length} invalid line(s); fix the producer, nothing was skipped silently`);
    return 1;
  }
  return 0;
}
