// The analysis pipeline shared by the CLI commands and the self-test.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { analyzeRun } from "./analyze.mjs";
import { MERGED_HEADER, RUNS_HEADER, mergedRows, runRows, toCsv } from "./csv.mjs";
import { readRuns } from "./read.mjs";
import { loadSchema } from "./schema.mjs";

export const DEFAULT_THRESHOLDS = fileURLToPath(new URL("../thresholds.json", import.meta.url));

export const loadThresholds = (path) => JSON.parse(readFileSync(path, "utf8"));

/** Analyzes every run found in `inputs`. */
export function analyzeInputs(inputs, thresholdsPath = DEFAULT_THRESHOLDS) {
  const { runs, problems } = readRuns(inputs, loadSchema());
  const thresholds = loadThresholds(thresholdsPath);
  const results = [...runs.values()].map((run) => ({ run, result: analyzeRun(run, thresholds) }));
  return { results, problems };
}

/** Human-readable verdict lines. */
export function formatResult({ run, result }) {
  const head = `${result.verdict.toUpperCase().padEnd(12)} ${run.runId} ${run.bench} audio=${run.audio} anchor=${run.anchor} devices=${run.devices.length}`;
  const extra = [];
  if (result.usefulSeparationM !== undefined) {
    extra.push(`  useful separation ${result.usefulSeparationM ?? "n/a"} m (convoy gap ${result.convoyGapM} m)`);
  }
  if (result.hopP95Ms !== undefined) extra.push(`  hop p95 ${result.hopP95Ms ?? "n/a"} ms`);
  const devices = result.devices.map((d) => `  ${String(d.labNode).padStart(5)} ${d.verdict.padEnd(12)} ${d.reasons.join("; ")}`);
  return [head, ...extra, ...devices].join("\n");
}

/** Writes `<out>/<runId>-merged.csv` per run and `<out>/runs.csv`; returns the paths. */
export function writeCsv(results, out) {
  mkdirSync(out, { recursive: true });
  const written = [];
  const allRuns = [];
  for (const { run, result } of results) {
    const path = join(out, `${run.runId}-merged.csv`);
    writeFileSync(path, toCsv(MERGED_HEADER, mergedRows(run)));
    written.push(path);
    allRuns.push(...runRows(run, result));
  }
  const runsPath = join(out, "runs.csv");
  writeFileSync(runsPath, toCsv(RUNS_HEADER, allRuns));
  written.push(runsPath);
  return written;
}

