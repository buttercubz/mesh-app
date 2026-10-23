// Per-run metrics and go / no-go verdicts against thresholds.json (DESIGN §12.2).
// A run merges every device file with the same runId; each device judges what it
// received, and the run verdict is noGo if any device says noGo, go if every device
// with data says go, and inconclusive otherwise.

/** Nearest-rank percentile of an ascending array. */
export function percentile(sorted, q) {
  if (sorted.length === 0) return null;
  const rank = Math.min(sorted.length, Math.max(1, Math.ceil(q * sorted.length)));
  return sorted[rank - 1];
}

const asc = (xs) => [...xs].filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
const round = (v, d = 3) => (v === null || v === undefined ? null : Math.round(v * 10 ** d) / 10 ** d);

/** Combines per-device verdicts into the run verdict. */
export function combine(verdicts) {
  if (verdicts.includes("noGo")) return "noGo";
  if (verdicts.length > 0 && verdicts.every((v) => v === "go")) return "go";
  return "inconclusive";
}

/** Link metrics of one device: from its summary line, or derived from its samples. */
export function linkMetrics(device) {
  const s = device.summary;
  if (s) {
    return {
      derived: false,
      durationS: s.durationS,
      txKbps: s.txKbps.mean,
      rxKbps: s.rxKbps?.mean ?? null,
      loss: s.loss,
      owdP95: s.owdMs?.p95 ?? null,
      rttP95: s.rttMs?.p95 ?? null,
      stalls: s.stalls,
      perLegMs: s.perLegMs ?? null,
    };
  }
  // Interrupted run: approximate from the 1 Hz samples.
  const links = device.samples.flatMap((x) => x.links);
  const mean = (xs) => (xs.length === 0 ? null : xs.reduce((a, b) => a + b, 0) / xs.length);
  return {
    derived: true,
    durationS: device.samples.at(-1)?.ts ?? 0,
    txKbps: mean(links.map((l) => l.txKbps)) ?? 0,
    rxKbps: mean(links.map((l) => l.rxKbps)),
    loss: mean(links.map((l) => l.loss)) ?? 0,
    owdP95: percentile(asc(links.map((l) => l.owdP95)), 0.95),
    rttP95: percentile(asc(links.map((l) => l.rttP95)), 0.95),
    stalls: links.reduce((n, l) => n + l.stalls, 0),
    perLegMs: null,
  };
}

/** Distance of a sample: `distM`, else computed from own and peer GPS. */
function sampleDistance(s) {
  if (typeof s.distM === "number") return s.distM;
  if (!s.gps || !s.peerGps?.length) return null;
  const R = 6_371_008.8;
  const rad = (d) => (d * Math.PI) / 180;
  return Math.min(
    ...s.peerGps.map((p) => {
      const dla = rad(p.lat - s.gps.lat);
      const dlo = rad(p.lon - s.gps.lon);
      const h = Math.sin(dla / 2) ** 2 + Math.cos(rad(s.gps.lat)) * Math.cos(rad(p.lat)) * Math.sin(dlo / 2) ** 2;
      return 2 * R * Math.asin(Math.min(1, Math.sqrt(h)));
    }),
  );
}

/**
 * T1 windows of one device. A 1 s window is valid only when the run's keep-alive anchor
 * was confirmed (`anchor.ok === true`); seconds without a sample line (the app was
 * suspended) and seconds whose anchor dropped are excluded, never counted as radio
 * loss. A valid window is good when a link was up and every link's loss < maxLoss.
 */
export function t1Windows(device, th) {
  const out = { valid: 0, good: 0, excludedAnchor: 0, missing: 0, windows: [] };
  let prevTs = 0;
  for (const s of device.samples) {
    out.missing += Math.max(0, Math.round(s.ts - prevTs) - 1);
    prevTs = s.ts;
    if (s.anchor?.ok !== true) {
      out.excludedAnchor += 1;
      continue;
    }
    const good = s.links.length > 0 && s.links.every((l) => l.loss < th.maxLoss);
    out.valid += 1;
    if (good) out.good += 1;
    out.windows.push({ ts: s.ts, good, distM: sampleDistance(s) });
  }
  return out;
}

/**
 * Useful separation: the largest distance D (in `binM` steps) such that at least
 * `share` of the valid windows recorded at ≤ D were good. `null` without distances.
 */
export function usefulSeparation(windows, share, binM) {
  const withDist = windows.filter((w) => typeof w.distM === "number");
  if (withDist.length === 0) return null;
  const maxD = Math.max(...withDist.map((w) => w.distM));
  let best = null;
  for (let d = binM; d <= Math.ceil(maxD / binM) * binM; d += binM) {
    const inRange = withDist.filter((w) => w.distM <= d);
    if (inRange.length === 0) continue;
    const ok = inRange.filter((w) => w.good).length / inRange.length;
    if (ok >= share) best = d;
  }
  return best === null ? 0 : Math.min(best, Math.ceil(maxD));
}

const go = (ok) => (ok ? "go" : "noGo");

/** Verdict of one device for a link bench (T0, T0c, T4). */
function linkVerdict(device, t, bench) {
  const m = linkMetrics(device);
  const p = device.run.params ?? {};
  const maxLoss = t.maxLoss;
  const owd = m.owdP95 ?? (m.rttP95 === null ? null : m.rttP95 / 2);
  const reasons = [];
  let v;
  if (m.durationS <= 0) v = "inconclusive";
  else if (p.pattern === "saturate") {
    const kbps = Math.max(m.txKbps, m.rxKbps ?? 0);
    v = go(m.loss < maxLoss && kbps >= (t.bleFastKbps ?? 0));
    reasons.push(`saturate ${round(kbps, 1)} kbps`);
  } else if (p.pattern === "idle") v = "inconclusive";
  else if (owd === null) v = m.loss >= maxLoss ? "noGo" : "inconclusive";
  else {
    const minStreams = bench === "T4" ? t.streams : (t.minStreams ?? 0);
    const streamsOk = p.pattern === "ping" || (p.streams ?? 0) >= minStreams;
    const owdLimit = t.owdP95Ms ?? Infinity;
    v = go(streamsOk && m.loss < maxLoss && owd < owdLimit);
    reasons.push(`loss ${round(m.loss * 100, 2)} %, owd p95 ${round(owd, 1)} ms, streams ${p.streams ?? 0}`);
    if (bench === "T0" && p.headset !== true) reasons.push("headset off: informative only for S_MAX_BLE");
  }
  if (m.derived) reasons.push("no summary line (interrupted run): derived from samples");
  return { verdict: v, reasons, metrics: m };
}

/** Per-device and merged verdict of one run. */
export function analyzeRun(run, thresholds) {
  const t = thresholds[run.bench] ?? {};
  const perDevice = [];
  const extra = {};
  switch (run.bench) {
    case "T0":
    case "T0c":
    case "T4":
      for (const d of run.devices) perDevice.push({ labNode: d.run.labNode, ...linkVerdict(d, t, run.bench) });
      break;
    case "T0b": {
      const hops = asc(
        run.devices.flatMap((d) => d.raws.flatMap((r) => (r.meshProbes ?? []).filter((m) => m.hops > 0).map((m) => m.ageMs / m.hops))),
      );
      const hopP95 = percentile(hops, 0.95);
      extra.hopP95Ms = round(hopP95, 2);
      for (const d of run.devices) {
        const m = linkMetrics(d);
        const v = hopP95 === null ? "inconclusive" : go(m.loss < t.maxLoss && hopP95 <= t.hopP95Ms);
        perDevice.push({ labNode: d.run.labNode, verdict: v, reasons: [`loss ${round(m.loss * 100, 2)} %, hop p95 ${extra.hopP95Ms} ms`], metrics: m });
      }
      break;
    }
    case "T1": {
      const all = [];
      for (const d of run.devices) {
        const w = t1Windows(d, t);
        all.push(...w.windows);
        const v = w.valid < t.minWindows ? "inconclusive" : go(w.good / w.valid >= t.windowOkShare);
        perDevice.push({
          labNode: d.run.labNode,
          verdict: v,
          reasons: [`${w.good}/${w.valid} valid windows good; excluded ${w.excludedAnchor} (anchor) + ${w.missing} (no sample)`],
          metrics: { valid: w.valid, good: w.good, excludedAnchor: w.excludedAnchor, missing: w.missing },
        });
      }
      const sep = usefulSeparation(all, t.windowOkShare, t.distanceBinM);
      extra.usefulSeparationM = sep;
      extra.convoyGapM = t.convoyGapM;
      if (sep !== null && combine(perDevice.map((d) => d.verdict)) === "go" && sep < t.convoyGapM) {
        perDevice.forEach((d) => {
          d.verdict = "noGo";
          d.reasons.push(`useful separation ${sep} m < convoy gap ${t.convoyGapM} m`);
        });
      }
      break;
    }
    case "T2":
      for (const d of run.devices) {
        const stops = d.events.filter((e) => e.ev === "captureStopped").length;
        const lost = d.events.filter((e) => e.ev === "captureLost").length;
        const silent = Math.max(0, stops - lost);
        const dur = linkMetrics(d).durationS;
        const v = dur < t.minDurationS ? "inconclusive" : go(silent <= t.maxSilentMutes);
        perDevice.push({ labNode: d.run.labNode, verdict: v, reasons: [`${silent} silent mutes (${stops} capture stops)`], metrics: { stops, lost } });
      }
      break;
    case "T3":
      for (const d of run.devices) {
        const rejoins = d.events.filter((e) => e.ev === "linkUp" && typeof e.reconnectS === "number").map((e) => e.reconnectS);
        const v = rejoins.length === 0 ? "inconclusive" : go(rejoins.every((s) => s <= t.rejoinS));
        perDevice.push({ labNode: d.run.labNode, verdict: v, reasons: [`rejoins ${rejoins.map((s) => round(s, 1)).join(", ") || "none"} s`], metrics: { rejoins } });
      }
      break;
    case "T5a":
      for (const d of run.devices) {
        const leg = linkMetrics(d).perLegMs;
        const v = leg === null ? "inconclusive" : go(leg <= t.legMs);
        perDevice.push({ labNode: d.run.labNode, verdict: v, reasons: [`per-leg ${leg ?? "n/a"} ms`], metrics: { perLegMs: leg } });
      }
      break;
    case "T6":
      for (const d of run.devices) {
        const pts = d.samples.filter((s) => typeof s.dev.bat === "number");
        const charging = pts.some((s) => s.dev.chg === true);
        const [a, b] = [pts[0], pts.at(-1)];
        const hours = a && b ? (b.ts - a.ts) / 3600 : 0;
        const rate = hours > 0 && !charging ? ((a.dev.bat - b.dev.bat) * 100) / hours : null;
        const dur = linkMetrics(d).durationS;
        const v = rate === null || dur < t.minDurationS ? "inconclusive" : go(rate < t.batteryPctPerH);
        perDevice.push({ labNode: d.run.labNode, verdict: v, reasons: [`battery ${rate === null ? "n/a" : round(rate, 2)} %/h`], metrics: { batteryPctPerH: rate } });
      }
      break;
    default:
      for (const d of run.devices) {
        perDevice.push({ labNode: d.run.labNode, verdict: "inconclusive", reasons: ["checklist / offline analysis (no automatic verdict)"], metrics: {} });
      }
  }
  return {
    runId: run.runId,
    bench: run.bench,
    audio: run.audio,
    anchor: run.anchor,
    devices: perDevice,
    verdict: combine(perDevice.map((d) => d.verdict)),
    ...extra,
  };
}
