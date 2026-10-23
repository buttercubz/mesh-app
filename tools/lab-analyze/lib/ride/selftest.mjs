// Self-test of `lab-analyze ride` (part of `--self-test`): merge by rideKey, clock
// alignment with injected ±3 s skew, traffic totals = sum of samples, glitch summary,
// the A1 permission/mic mismatch, ZIP and concatenated inputs, the labrun/1
// projection and schema enforcement (an invalid line is reported, exit code 1).
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { loadRideSchema, loadSchema, validate } from "../schema.mjs";
import { analyzeRides, rideCommand, writeAsLabrun } from "./index.mjs";
import { readRideSources, readRides } from "./read.mjs";
import { PHONES, RIDE_KEY, TRUE_START, phoneLines, phoneText } from "./synth.mjs";
import { writeZip } from "./zip.mjs";

const FIXTURES = fileURLToPath(new URL("../../fixtures/ride", import.meta.url));

export function rideSelfTest(check) {
  const schema = loadRideSchema();
  // 1. Every synthetic line validates; the committed fixtures are the same ride.
  for (const p of PHONES) {
    const bad = phoneLines(p).flatMap((l) => validate(l, schema).map((e) => `${l.t}${e.path} ${e.message}`));
    check(bad.length === 0, `synthetic ${p.model} lines validate (${bad.slice(0, 3).join("; ")})`);
    const fixture = readFileSync(join(FIXTURES, `ride-${p.node}.jsonl`), "utf8");
    check(fixture === phoneText(p), `fixtures/ride-${p.node}.jsonl matches the generator`);
  }
  const other = { ...PHONES[0], sid: "d-20261022-2004-zzzz" };
  const sources = [
    ...PHONES.map((p) => ({ name: `ride-${p.node}.jsonl`, text: phoneText(p) })),
    { name: "other.jsonl", text: phoneText(other, { rideKey: "0badc0de" }) },
  ];
  const { sessions, problems } = readRideSources(sources, schema);
  check(problems.length === 0, `synthetic ride reads cleanly (${problems.join("; ")})`);
  const results = analyzeRides(sessions, {});
  const ride = results.find((r) => r.rideKey === RIDE_KEY);
  check(results.length === 2 && ride?.ride.sessions.length === 3, "files merge by rideKey (3 phones + 1 other ride)");
  check(analyzeRides(sessions, { ride: RIDE_KEY }).length === 1, "--ride keeps one ride");
  if (!ride) return;

  // 2. Alignment: wall clocks skewed by +3 s and -2.5 s are corrected via clkOffset.
  for (const p of PHONES) {
    const a = ride.alignment.find((x) => x.sessionId === p.sid);
    check(Math.abs(a.corrMs + p.skewMs) <= 1, `${p.model} corrected by ${-p.skewMs} ms (got ${a.corrMs})`);
  }
  const down = ride.timeline.filter((t) => t.what === "linkDown");
  const spread = Math.max(...down.map((t) => t.wall)) - Math.min(...down.map((t) => t.wall));
  check(down.length === 4 && spread <= 5, `the 4 linkDown ends line up after alignment (spread ${spread} ms)`);
  check(down.every((t) => t.wall === TRUE_START + 60_000), "linkDown at the true instant");

  // 3. Traffic totals equal the sum of the samples; idle overhead is non-voice only.
  const tab = ride.ride.sessions.find((s) => s.node === 101);
  const sumB = tab.samples.reduce((n, s) => n + s.links.filter((l) => l.id === 1).reduce((m, l) => m + l.tx.bytes, 0), 0);
  const rows = ride.traffic.links.filter((l) => l.node === 101 && l.link === 1);
  const fromRows = rows.reduce((n, r) => n + (r.txKbps * 1000 * secsOf(tab, r.minute)) / 8, 0);
  check(Math.abs(fromRows - sumB) < 2 * rows.length, `traffic table total ${fromRows} B = sum of samples ${sumB} B`);
  check(rows.every((r) => r.farTxKbps !== null), "both ends of link 1 matched");
  const idle = ride.traffic.idle;
  check(
    idle.every((o) => o.voiceKbps === 0) && idle.filter((o) => o.node === 101).every((o) => o.nonVoiceKbps > 0),
    "idle overhead counts non-voice bytes only",
  );

  // 4. Audio: the Tab's crackle, the Galaxy's mismatch (scenario A1).
  const aTab = ride.audio.find((a) => a.device === tab.label);
  const under = aTab.underPerMin.reduce((n, m) => n + m.under, 0);
  check(under === 6 && aTab.cbSizeChanges === 3 && aTab.glitches === 3, `Tab glitch summary (under ${under}, cb ${aTab.cbSizeChanges})`);
  check(aTab.plcPer1000 !== null && aTab.plcPer1000 > 0, "PLC per 1000 frames reported");
  const aGal = ride.audio.find((a) => a.device.startsWith("n202"));
  check(/perm granted .* micState=paused cause=builtInMicOff/.test(aGal.mismatch ?? ""), "A1 mismatch found on the Galaxy");
  check(ride.partitions.islands.length === 3, "partition events from the 3 phones");
  check(ride.marks.length === 1 && ride.marks[0].context.some((c) => c.what === "heard"), "mark with ±10 s context");

  const dir = mkdtempSync(join(tmpdir(), "lab-analyze-ride-"));
  try {
    // 5. ZIP export (deflated, nested segments) + concatenated segments read the same.
    const zip = writeZip(
      [
        { name: "device.json", data: "{}" },
        ...PHONES.slice(0, 2).map((p) => ({ name: `diag/${p.sid}/seg-0001.jsonl`, data: phoneText(p) })),
      ],
      { deflate: true },
    );
    writeFileSync(join(dir, "export.zip"), zip);
    writeFileSync(join(dir, "iphone.jsonl"), phoneText(PHONES[2]));
    const fromFiles = readRides([dir], schema);
    check(fromFiles.problems.length === 0 && fromFiles.sessions.length === 3, "ZIP + JSONL inputs read 3 sessions");

    // 6. --as-labrun: the projection validates against labrun/1.
    const lab = loadSchema();
    const paths = writeAsLabrun([ride], join(dir, "labrun"));
    let labBad = 0;
    for (const p of paths) {
      for (const l of readFileSync(p, "utf8").trim().split("\n")) labBad += validate(JSON.parse(l), lab).length;
    }
    check(paths.length === 3 && labBad === 0, `labrun/1 projection validates (${labBad} errors)`);

    // 7. An invalid line is reported and makes the command fail.
    writeFileSync(join(dir, "bad.jsonl"), `${phoneText(PHONES[0]).trim()}\n{"t":"e","ts":1,"src":"core","ev":"Bad Name"}\n`);
    const log = console.log;
    const err = console.error;
    let code;
    try {
      console.log = () => {};
      console.error = () => {};
      code = rideCommand({ inputs: [join(dir, "bad.jsonl")] });
    } finally {
      console.log = log;
      console.error = err;
    }
    check(code === 1, "an invalid ridelog line makes `ride` exit 1");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  console.log(`ride self-test: ${ride.timeline.length} timeline rows, ${ride.traffic.links.length} traffic rows`);
}

function secsOf(s, minute) {
  return s.samples.filter((x) => {
    const d = new Date(s.startedMs + x.ts * 1000 + (s.corrMs ?? 0));
    return d.toISOString().slice(11, 16) === minute;
  }).length;
}
