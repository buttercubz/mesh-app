#!/usr/bin/env node
// lab-analyze (WP-05, DESIGN §12.3): merges per-device labrun/1 files by runId, writes
// CSV and prints go / no-go verdicts against thresholds.json.
//
//   node tools/lab-analyze/cli.mjs merge <dir|file...> [--out <dir>] [--thresholds <file>]
//   node tools/lab-analyze/cli.mjs verdicts <dir|file...> [--thresholds <file>] [--json] [--strict]
//   node tools/lab-analyze/cli.mjs validate <dir|file...>
//   node tools/lab-analyze/cli.mjs ride <zip|dir|jsonl...> [--ride <rideKey>] [--from HH:MM --to HH:MM]
//                                   [--all] [--json] [--csv <dir>] [--as-labrun <dir>]
//   node tools/lab-analyze/cli.mjs --self-test
//
// `ride` reads ridelog/1 diagnostics exports (diagnostics flight recorder), merges the
// phones of one ride by rideKey and prints the timeline, per-link traffic, audio and
// partition report; times are UTC.
//
// Exit codes: 0 ok; 1 a validation error, a failed self-test or (with --strict) a noGo
// run; 2 usage error.
import { statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { DEFAULT_THRESHOLDS, analyzeInputs, formatResult, writeCsv } from "./lib/pipeline.mjs";
import { readRuns } from "./lib/read.mjs";
import { rideCommand } from "./lib/ride/index.mjs";
import { loadSchema } from "./lib/schema.mjs";
import { selfTest } from "./lib/selftest.mjs";

const USAGE = `usage:
  lab-analyze merge <dir|file...> [--out <dir>] [--thresholds <file>]
  lab-analyze verdicts <dir|file...> [--thresholds <file>] [--json] [--strict]
  lab-analyze validate <dir|file...>
  lab-analyze ride <zip|dir|jsonl...> [--ride <rideKey>] [--from HH:MM --to HH:MM] [--all] [--json] [--csv <dir>] [--as-labrun <dir>]
  lab-analyze --self-test`;

class UsageError extends Error {}

function parseArgs(argv) {
  const opts = {
    inputs: [],
    out: null,
    thresholds: DEFAULT_THRESHOLDS,
    json: false,
    strict: false,
    ride: null,
    from: null,
    to: null,
    all: false,
    csv: null,
    asLabrun: null,
  };
  let cmd = null;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) throw new UsageError(`${a} needs a value`);
      return argv[++i];
    };
    if (a === "--self-test") cmd = "self-test";
    else if (a === "--out") opts.out = value();
    else if (a === "--thresholds") opts.thresholds = value();
    else if (a === "--json") opts.json = true;
    else if (a === "--strict") opts.strict = true;
    else if (a === "--ride") opts.ride = value();
    else if (a === "--from") opts.from = value();
    else if (a === "--to") opts.to = value();
    else if (a === "--all") opts.all = true;
    else if (a === "--csv") opts.csv = value();
    else if (a === "--as-labrun") opts.asLabrun = value();
    else if (a === "--help" || a === "-h") cmd = "help";
    else if (a.startsWith("--")) throw new UsageError(`unknown option ${a}`);
    else if (cmd === null) cmd = a;
    else opts.inputs.push(a);
  }
  if (cmd === null) throw new UsageError("missing command");
  if (["merge", "verdicts", "validate", "ride"].includes(cmd) && opts.inputs.length === 0) {
    throw new UsageError(`${cmd} needs at least one directory or file`);
  }
  if (!["merge", "verdicts", "validate", "ride", "self-test", "help"].includes(cmd)) {
    throw new UsageError(`unknown command ${cmd}`);
  }
  return { cmd, opts };
}

function main(argv) {
  let parsed;
  try {
    parsed = parseArgs(argv);
  } catch (e) {
    if (!(e instanceof UsageError)) throw e;
    console.error(`lab-analyze: ${e.message}\n${USAGE}`);
    return 2;
  }
  const { cmd, opts } = parsed;
  switch (cmd) {
    case "help":
      console.log(USAGE);
      return 0;
    case "self-test":
      return selfTest() ? 0 : 1;
    case "ride":
      return rideCommand(opts);
    case "validate": {
      const { runs, problems } = readRuns(opts.inputs, loadSchema());
      for (const p of problems) console.error(p);
      const files = [...runs.values()].reduce((n, r) => n + r.devices.length, 0);
      console.log(`${files} file(s) in ${runs.size} run(s); ${problems.length} problem(s)`);
      return problems.length === 0 ? 0 : 1;
    }
    case "merge":
    case "verdicts": {
      const { results, problems } = analyzeInputs(opts.inputs, opts.thresholds);
      for (const p of problems) console.error(`warning: ${p}`);
      if (cmd === "merge") {
        const first = opts.inputs[0];
        const out = opts.out ?? join(statSync(first).isDirectory() ? first : dirname(first), "analysis");
        for (const p of writeCsv(results, out)) console.log(`wrote ${p}`);
      }
      if (opts.json) console.log(JSON.stringify(results.map((r) => r.result), null, 2));
      else for (const r of results) console.log(formatResult(r));
      const noGo = results.some((r) => r.result.verdict === "noGo");
      return opts.strict && noGo ? 1 : 0;
    }
    default:
      return 2;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  process.exitCode = main(process.argv.slice(2));
}
