// `--self-test`: merges the fixture files (two devices per run, written by
// `cargo run -p mesh-lab --example lab_fixtures`), prints their go / no-go verdicts and
// checks the analysis rules: schema enforcement, the merge by runId, the T1 exclusion
// of windows whose anchor dropped, CSV output and interrupted-run handling; then the
// ridelog/1 `ride` report (lib/ride/selftest.mjs).
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { analyzeRun } from "./analyze.mjs";
import { analyzeInputs, formatResult, loadThresholds, writeCsv } from "./pipeline.mjs";
import { parseDeviceFile } from "./read.mjs";
import { rideSelfTest } from "./ride/selftest.mjs";
import { loadSchema, validate } from "./schema.mjs";

const FIXTURES = fileURLToPath(new URL("../fixtures", import.meta.url));

export function selfTest() {
  const failures = [];
  const check = (ok, what) => {
    if (!ok) failures.push(what);
  };
  const schema = loadSchema();
  const thresholds = loadThresholds(fileURLToPath(new URL("../thresholds.json", import.meta.url)));

  // 1. Schema: the fixtures validate; the mandatory audio/anchor and anchor/bt fields
  //    are enforced.
  const t0a = readFileSync(join(FIXTURES, "t0-device-a.jsonl"), "utf8");
  const runLine = JSON.parse(t0a.split("\n")[0]);
  const sampleLine = JSON.parse(t0a.split("\n").find((l) => l.includes('"t":"s"')));
  check(validate(runLine, schema).length === 0, "the fixture run line validates");
  for (const field of ["audio", "anchor"]) {
    const bad = { ...runLine };
    delete bad[field];
    check(validate(bad, schema).length > 0, `a run line without ${field} is rejected`);
  }
  for (const field of ["anchor", "bt"]) {
    const bad = { ...sampleLine };
    delete bad[field];
    check(validate(bad, schema).length > 0, `a sample without ${field} is rejected`);
  }
  const torn = parseDeviceFile(`${JSON.stringify({ ...runLine, audio: undefined })}\n`, "torn.jsonl", schema);
  check(torn.device === null && torn.problems.length > 0, "a file whose run line lacks audio is skipped");

  // 2. Merge the two fixture pairs by runId and print the verdicts.
  const { results, problems } = analyzeInputs([FIXTURES]);
  check(problems.length === 0, `fixtures read without problems (${problems.join("; ")})`);
  check(results.length === 2, `two runs merged (got ${results.length})`);
  for (const r of results) {
    console.log(formatResult(r));
    check(r.run.devices.length === 2, `${r.run.runId} merges two device files`);
  }
  const t0 = results.find((r) => r.run.bench === "T0");
  const t1 = results.find((r) => r.run.bench === "T1");
  check(t0?.result.verdict === "go", "T0 fixture (2 streams, 20 ms one way, no loss) is go");

  // 3. T1: windows whose anchor dropped are excluded from the range verdict.
  check(t1?.result.verdict === "go", "T1 fixture is go with the dropped-anchor windows excluded");
  for (const d of t1?.result.devices ?? []) {
    check(d.metrics.excludedAnchor >= 30, `T1 device ${d.labNode} excluded ≥ 30 anchor-dropped windows`);
  }
  check((t1?.result.usefulSeparationM ?? 0) >= thresholds.T1.convoyGapM, "T1 useful separation reaches the convoy gap");
  if (t1) {
    // Control: the same seconds counted as radio loss (anchor held, no link) fail T1.
    const control = structuredClone(t1.run);
    for (const d of control.devices) {
      for (const s of d.samples) {
        if (s.anchor.ok !== true) {
          s.anchor.ok = true;
          s.links = [];
        }
      }
    }
    check(analyzeRun(control, thresholds).verdict === "noGo", "the same gap with the anchor held is noGo");
  }

  // 4. CSV output.
  const out = mkdtempSync(join(tmpdir(), "lab-analyze-"));
  try {
    const written = writeCsv(results, out);
    check(written.length === 3, "one merged CSV per run plus runs.csv");
    const merged = readFileSync(written.find((p) => p.includes(t1?.run.runId ?? "?")), "utf8").trim().split("\n");
    const nodes = new Set(merged.slice(1).map((l) => l.split(",")[2]));
    check(nodes.size === 2, "the merged CSV interleaves both devices");
    const runsCsv = readFileSync(join(out, "runs.csv"), "utf8").trim().split("\n");
    check(runsCsv.length === 5, "runs.csv has one row per run and device");
  } finally {
    rmSync(out, { recursive: true, force: true });
  }

  // 5. An interrupted run (no summary line) is still judged from its samples.
  if (t0) {
    const cut = structuredClone(t0.run);
    cut.devices.forEach((d) => {
      d.summary = null;
    });
    const r = analyzeRun(cut, thresholds);
    check(r.devices.every((d) => d.metrics.derived), "metrics derived from samples without a summary");
    check(r.verdict === "go", "the interrupted T0 run is still go");
  }

  // 6. ridelog/1: `lab-analyze ride` (diagnostics flight recorder).
  rideSelfTest(check);

  for (const f of failures) console.error(`self-test FAILED: ${f}`);
  const verdicts = results.map((r) => `${r.run.bench}=${r.result.verdict}`).join(" ");
  console.log(failures.length === 0 ? `self-test PASS (${verdicts})` : `self-test FAIL (${failures.length})`);
  return failures.length === 0;
}
