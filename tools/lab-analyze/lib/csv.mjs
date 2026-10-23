// CSV output: per-run merged 1 Hz rows (every device, every link) and one row per
// run and device with its verdict.

/** RFC 4180 field quoting. */
export function csvField(v) {
  if (v === null || v === undefined) return "";
  const s = String(v);
  return /[",\r\n]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s;
}

export const toCsv = (header, rows) =>
  `${[header, ...rows].map((r) => r.map(csvField).join(",")).join("\n")}\n`;

export const MERGED_HEADER = [
  "runId",
  "bench",
  "labNode",
  "ts",
  "link",
  "kind",
  "peer",
  "rssi",
  "txKbps",
  "rxKbps",
  "loss",
  "rttP50",
  "rttP95",
  "owdP95",
  "stalls",
  "distM",
  "speed",
  "anchorOk",
  "bt",
  "app",
  "locked",
  "battery",
  "therm",
  "route",
  "ladder",
  "mark",
];

/** Merged 1 Hz rows of a run, ordered by time then device then link. */
export function mergedRows(run) {
  const rows = [];
  for (const d of run.devices) {
    const marks = d.marks.map((m) => ({ ts: m.ts, label: m.label }));
    for (const s of d.samples) {
      const mark = marks.filter((m) => m.ts > s.ts - 1 && m.ts <= s.ts).map((m) => m.label).join("|");
      const base = (l) => [
        run.runId,
        run.bench,
        d.run.labNode,
        s.ts,
        l?.id,
        l?.kind,
        l?.peer,
        l?.rssi,
        l?.txKbps,
        l?.rxKbps,
        l?.loss,
        l?.rttP50,
        l?.rttP95,
        l?.owdP95,
        l?.stalls,
        s.distM,
        s.gps?.spd,
        s.anchor?.ok,
        s.bt,
        s.dev.app,
        s.dev.locked,
        s.dev.bat,
        s.dev.therm,
        s.audio.route,
        s.audio.ladder,
        mark,
      ];
      if (s.links.length === 0) rows.push(base(null));
      else for (const l of s.links) rows.push(base(l));
    }
  }
  rows.sort((a, b) => a[3] - b[3] || a[2] - b[2] || (a[4] ?? 0) - (b[4] ?? 0));
  return rows;
}

export const RUNS_HEADER = [
  "runId",
  "bench",
  "audio",
  "anchor",
  "labNode",
  "model",
  "role",
  "durationS",
  "txKbps",
  "loss",
  "owdP95",
  "rttP95",
  "deviceVerdict",
  "runVerdict",
  "reasons",
];

/** One row per run and device. */
export function runRows(run, result) {
  return run.devices.map((d) => {
    const r = result.devices.find((x) => x.labNode === d.run.labNode);
    const s = d.summary;
    return [
      run.runId,
      run.bench,
      run.audio,
      run.anchor,
      d.run.labNode,
      d.run.device.model,
      d.run.role,
      s?.durationS,
      s?.txKbps?.mean,
      s?.loss,
      s?.owdMs?.p95,
      s?.rttMs?.p95,
      r?.verdict,
      result.verdict,
      r?.reasons.join("; "),
    ];
  });
}
