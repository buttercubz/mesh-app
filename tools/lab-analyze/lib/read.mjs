// Reads labrun/1 files (one per device per run) and groups them by runId.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, join } from "node:path";

import { validate } from "./schema.mjs";

/** Expands directories into their *.jsonl files (sorted, non-recursive). */
export function listInputs(paths) {
  const files = [];
  for (const p of paths) {
    if (statSync(p).isDirectory()) {
      for (const name of readdirSync(p).sort()) {
        if (name.endsWith(".jsonl")) files.push(join(p, name));
      }
    } else {
      files.push(p);
    }
  }
  return files;
}

/**
 * Parses one device file. Lines that are not JSON or fail the schema are reported in
 * `problems` and skipped; a file whose first valid line is not a run line is rejected.
 */
export function parseDeviceFile(text, name, schema) {
  const problems = [];
  const lines = [];
  text.split(/\r?\n/).forEach((raw, i) => {
    if (raw.trim() === "") return;
    let value;
    try {
      value = JSON.parse(raw);
    } catch (e) {
      // A torn last line is normal for an interrupted run; report it and go on.
      problems.push(`${name}:${i + 1}: not JSON (${e.message})`);
      return;
    }
    if (schema) {
      const errors = validate(value, schema);
      if (errors.length > 0) {
        problems.push(`${name}:${i + 1}: ${errors.map((e) => `${e.path} ${e.message}`).join("; ")}`);
        return;
      }
    }
    lines.push(value);
  });
  const run = lines.find((l) => l.t === "run");
  if (!run || lines[0] !== run) {
    problems.push(`${name}: the first line is not a valid run line; file skipped`);
    return { device: null, problems };
  }
  const device = {
    file: name,
    run,
    samples: lines.filter((l) => l.t === "s"),
    events: lines.filter((l) => l.t === "e"),
    marks: lines.filter((l) => l.t === "mark"),
    raws: lines.filter((l) => l.t === "raw"),
    summary: lines.find((l) => l.t === "summary") ?? null,
  };
  return { device, problems };
}

/** Reads every input and returns `{ runs: Map<runId, Run>, problems }`. */
export function readRuns(paths, schema) {
  const runs = new Map();
  const problems = [];
  for (const file of listInputs(paths)) {
    const r = parseDeviceFile(readFileSync(file, "utf8"), basename(file), schema);
    problems.push(...r.problems);
    if (r.device) addDevice(runs, r.device, problems);
  }
  return { runs, problems };
}

/** Groups a parsed device file into its run (all devices must agree on bench/audio/anchor). */
export function addDevice(runs, device, problems) {
  const id = device.run.runId;
  let run = runs.get(id);
  if (!run) {
    run = { runId: id, bench: device.run.bench, audio: device.run.audio, anchor: device.run.anchor, devices: [] };
    runs.set(id, run);
  }
  if (device.run.bench !== run.bench) {
    problems.push(`${device.file}: bench ${device.run.bench} differs from ${run.bench} in run ${id}; file skipped`);
    return;
  }
  if (run.devices.some((d) => d.run.labNode === device.run.labNode)) {
    problems.push(`${device.file}: labNode ${device.run.labNode} appears twice in run ${id}; file skipped`);
    return;
  }
  run.devices.push(device);
  run.devices.sort((a, b) => a.run.labNode - b.run.labNode);
}
