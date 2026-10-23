// Reads ridelog/1 inputs (segment files, concatenated exports, ZIP exports,
// directories) into per-device sessions. Every line is validated against the schema;
// a problem is reported (and makes `lab-analyze ride` exit 1), never silently skipped.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import { gunzipSync } from "node:zlib";

import { validate } from "../schema.mjs";
import { readZip } from "./zip.mjs";

/** Expands paths into `{ name, text }` sources (directories recursively). */
export function loadSources(paths) {
  const out = [];
  const add = (name, buf) => {
    if (name.endsWith(".zip")) {
      for (const e of readZip(buf)) {
        if (e.name.endsWith(".jsonl") || e.name.endsWith(".jsonl.gz")) add(`${name}!${e.name}`, e.data);
      }
    } else if (name.endsWith(".gz")) {
      out.push({ name, text: gunzipSync(buf).toString("utf8") });
    } else {
      out.push({ name, text: buf.toString("utf8") });
    }
  };
  const walk = (p) => {
    if (statSync(p).isDirectory()) {
      for (const entry of readdirSync(p).sort()) {
        const child = join(p, entry);
        if (statSync(child).isDirectory()) walk(child);
        else if (/\.(jsonl|jsonl\.gz|zip)$/.test(entry)) add(child, readFileSync(child));
      }
    } else {
      add(p, readFileSync(p));
    }
  };
  for (const p of paths) walk(p);
  return out;
}

function newSession(hdr) {
  return {
    sessionId: hdr.sessionId,
    files: new Set(),
    headers: [],
    device: hdr.device,
    startedAt: hdr.startedAt,
    startedMs: Date.parse(hdr.startedAt),
    monoUs: hdr.monoUs,
    rideKey: null,
    node: null,
    role: null,
    samples: [],
    events: [],
    marks: [],
    frames: [],
    ends: [],
  };
}

/** Parses one source's text into `sessions` (a Map keyed by sessionId). */
export function parseSource(src, schema, sessions, problems) {
  let current = null;
  src.text.split(/\r?\n/).forEach((raw, i) => {
    if (raw.trim() === "") return;
    const where = `${basename(src.name)}:${i + 1}`;
    let v;
    try {
      v = JSON.parse(raw);
    } catch (e) {
      problems.push(`${where}: not JSON (${e.message})`);
      return;
    }
    const errors = schema ? validate(v, schema) : [];
    if (errors.length > 0) {
      problems.push(`${where}: ${errors.slice(0, 4).map((e) => `${e.path} ${e.message}`).join("; ")}`);
      return;
    }
    if (v.t === "hdr") {
      current = sessions.get(v.sessionId);
      if (!current) {
        current = newSession(v);
        sessions.set(v.sessionId, current);
      }
      current.files.add(src.name);
      current.headers.push(v);
      if (v.rideKey) current.rideKey = v.rideKey;
      if (v.node !== null) current.node = v.node;
      if (v.role) current.role = v.role;
      return;
    }
    if (!current) {
      problems.push(`${where}: "${v.t}" line before any hdr line`);
      return;
    }
    switch (v.t) {
      case "s":
        current.samples.push(v);
        break;
      case "e":
        current.events.push(v);
        if (v.ev === "session") {
          if (v.rideKey) current.rideKey = v.rideKey;
          if (v.node !== null && v.node !== undefined) current.node = v.node;
          if (v.role) current.role = v.role;
        }
        break;
      case "mark":
        current.marks.push(v);
        break;
      case "f":
        current.frames.push(v);
        break;
      case "end":
        current.ends.push(v);
        break;
      default:
        problems.push(`${where}: unknown line type ${v.t}`);
    }
  });
}

/** Reads every input; returns `{ sessions: Session[], problems }` sorted by start. */
export function readRides(paths, schema) {
  const sessions = new Map();
  const problems = [];
  for (const src of loadSources(paths)) parseSource(src, schema, sessions, problems);
  return { sessions: finish(sessions), problems };
}

/** Same as `readRides` for in-memory sources (`[{ name, text }]`). */
export function readRideSources(sources, schema) {
  const sessions = new Map();
  const problems = [];
  for (const src of sources) parseSource(src, schema, sessions, problems);
  return { sessions: finish(sessions), problems };
}

function finish(map) {
  const byTs = (a, b) => a.ts - b.ts;
  const list = [...map.values()];
  for (const s of list) {
    // Segments may arrive in any file order; lines of one session share one clock.
    s.samples.sort(byTs);
    s.events.sort(byTs);
    s.marks.sort(byTs);
    s.frames.sort(byTs);
    s.files = [...s.files];
    s.label = deviceLabel(s);
  }
  return list.sort((a, b) => a.startedMs - b.startedMs || a.sessionId.localeCompare(b.sessionId));
}

/** Short device label: node id when known, else the model. */
export function deviceLabel(s) {
  const who = s.node !== null ? `n${s.node}` : s.sessionId.slice(-4);
  return `${who}/${s.device.platform}`;
}
