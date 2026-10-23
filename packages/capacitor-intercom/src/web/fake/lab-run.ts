// One Lab run of the fake peer: records `labrun/1` JSON Lines (DESIGN §12.3, contract
// C7) and the one-row summary CSV, and computes the on-device `LabSummary`. Device and
// peer fields say "web"/"fake-peer" so a fake export can never pass for a phone run.
import type { LabLiveSample, LabRunInfo, LabSummary, LabVerdict } from "@meshapp/engine-api";

import type { ResolvedLabParams } from "./lab-params.js";
import type { LinkSecond } from "./lab-traffic.js";
import { percentile } from "./lab-traffic.js";

/** Directory of the fake Lab files (stand-in for Documents/lab). */
export const LAB_DIR = "lab";

export interface RunIdentity {
  runId: string;
  startedAt: string;
  labNode: number;
  peerLabNode: number | null;
  appVersion: string;
}

/** Per-second record handed to the run by the engine tick. */
export interface RunSecond {
  ts: number;
  link: LinkSecond | null;
  gps: { lat: number; lon: number; spd: number; acc: number } | null;
  peerGps: { lat: number; lon: number } | null;
  distM: number | null;
  anchorOk: boolean;
  background: boolean;
  peers: number;
}

const round3 = (v: number): number => Math.round(v * 1000) / 1000;

export class LabRun {
  readonly id: RunIdentity;
  readonly params: ResolvedLabParams;
  readonly #lines: string[] = [];
  readonly #seconds: RunSecond[] = [];
  #nextProbeId = 0;
  #summary: LabSummary | null = null;

  constructor(id: RunIdentity, params: ResolvedLabParams) {
    this.id = id;
    this.params = params;
    const p = params;
    this.#push({
      t: "run",
      v: 1,
      runId: id.runId,
      bench: p.bench,
      variant: {
        transport: p.transport,
        role: p.role,
        headset: p.headset ? "hfp" : "off",
        pattern: p.pattern,
        mixable: p.mixable,
        ...(p.variant === null ? {} : { label: p.variant }),
      },
      audio: p.audio,
      anchor: p.anchor,
      startedAt: id.startedAt,
      device: { model: "web", os: "browser", app: id.appVersion, platform: "web" },
      labNode: id.labNode,
      role: "controller",
      peers: id.peerLabNode === null ? [] : [{ labNode: id.peerLabNode, model: "fake-peer" }],
      params: {
        durationS: p.durationS,
        streams: p.streams,
        payloadB: p.payloadB,
        intervalMs: p.intervalMs,
        pingHz: p.pingHz,
        gps: p.gps,
      },
    });
  }

  get finished(): boolean {
    return this.#summary !== null;
  }

  get summary(): LabSummary | null {
    return this.#summary;
  }

  /** Paths of the files this run writes. */
  get files(): string[] {
    return [`${LAB_DIR}/${this.id.runId}.jsonl`, `${LAB_DIR}/${this.id.runId}-summary.csv`];
  }

  /** Records one 1 Hz sample (`s` line) and a `raw` line with the second's pings. */
  addSecond(s: RunSecond): void {
    this.#seconds.push(s);
    const link = s.link;
    this.#push({
      t: "s",
      ts: round3(s.ts),
      links:
        link === null
          ? []
          : [
              {
                id: 1,
                kind: `ble-${this.params.transport === "l2cap" ? "l2cap" : "gatt"}`,
                role: this.params.role,
                rssi: link.rssi,
                txKbps: link.txKbps,
                rxKbps: link.rxKbps,
                loss: link.loss,
                rttP50: link.rttP50,
                rttP95: link.rttP95,
                owdP95: link.owdP95,
                stalls: link.stalls,
                maxWrite: null,
              },
            ],
      gps: s.gps,
      peerGps:
        s.peerGps === null || this.id.peerLabNode === null
          ? []
          : [{ labNode: this.id.peerLabNode, ...s.peerGps }],
      distM: s.distM,
      audio: {
        running: this.params.audio !== "off",
        route: this.params.audio === "off" ? "none" : "builtinSpeaker",
        ladder: "L0",
        xruns: 0,
      },
      dev: { bat: null, chg: null, therm: "nominal", app: s.background ? "background" : "foreground", locked: false },
      // Mandatory anchor and radio state (§12.1). A browser has no location session or
      // Live Activity; the audio session holds whenever the run's anchor holds.
      anchor: { ok: s.anchorOk, session: s.anchorOk, location: false, liveActivity: false },
      bt: "on",
    });
    if (link !== null && link.pings.length > 0) {
      this.#push({ t: "raw", ts: round3(s.ts), link: 1, pings: this.#pingTuples(s.ts, link.pings) });
    }
  }

  /**
   * The second's pings as `[probeId, t1, t4, hold]` (run clock, s): probes spread evenly
   * over the second that ends at `ts`, each answered after its RTT, no peer hold time.
   */
  #pingTuples(ts: number, rttsMs: readonly number[]): [number, number, number, number][] {
    const n = rttsMs.length;
    return rttsMs.map((rtt, i) => {
      const t1 = Math.max(0, ts - 1 + i / n);
      const id = this.#nextProbeId;
      this.#nextProbeId += 1;
      return [id, round3(t1), round3(t1 + rtt / 1000), 0];
    });
  }

  /** Records an event line (`linkUp`, `linkDown`, `anchorLost` …). */
  event(ts: number, ev: string, fields: Record<string, string | number> = {}): void {
    this.#push({ t: "e", ts: round3(ts), ev, ...fields });
  }

  mark(ts: number, label: string): void {
    this.#push({ t: "mark", ts: round3(ts), label });
  }

  /** Live numbers for `labProgress` from the latest second. */
  liveSample(peers: number): LabLiveSample {
    const last = this.#seconds.at(-1);
    const link = last?.link ?? null;
    return {
      links: link === null ? 0 : 1,
      peers,
      txKbps: link?.txKbps ?? 0,
      rxKbps: link?.rxKbps ?? 0,
      loss: link?.loss ?? 0,
      rttP50Ms: link?.rttP50 ?? null,
      rttP95Ms: link?.rttP95 ?? null,
      rssi: link?.rssi ?? null,
      distM: last?.distM ?? null,
      stalls: this.#seconds.reduce((n, s) => n + (s.link?.stalls ?? 0), 0),
      anchorOk: last?.anchorOk ?? true,
    };
  }

  /** Ends the run normally: computes the summary and writes the `summary` line. */
  finish(durationS: number): LabSummary {
    const linked = this.#seconds.filter((s) => s.link !== null);
    const links = linked.map((s) => s.link as LinkSecond);
    const rtts = links.flatMap((l) => l.pings).sort((a, b) => a - b);
    const owds = links.map((l) => l.owdP95).sort((a, b) => a - b);
    const mean = (xs: number[]): number => (xs.length === 0 ? 0 : xs.reduce((a, b) => a + b, 0) / xs.length);
    const pct = (xs: number[]) =>
      xs.length === 0
        ? null
        : { p50: percentile(xs, 0.5), p95: percentile(xs, 0.95), p99: percentile(xs, 0.99) };
    const summary: LabSummary = {
      durationS: Math.round(durationS * 10) / 10,
      txKbps: { mean: Math.round(mean(links.map((l) => l.txKbps)) * 10) / 10 },
      loss: Math.round(mean(links.map((l) => l.loss)) * 10_000) / 10_000,
      owdMs: pct(owds),
      rttMs: pct(rtts),
      stalls: links.reduce((n, l) => n + l.stalls, 0),
      verdict: "inconclusive",
    };
    summary.verdict = verdictOf(this.params, this.#seconds, summary);
    this.#summary = summary;
    this.#push({ t: "summary", ...summary });
    return summary;
  }

  /** The `labrun/1` JSONL file content. */
  jsonl(): string {
    return `${this.#lines.join("\n")}\n`;
  }

  /** The `…-summary.csv` content (header + one row). */
  csv(): string {
    const s = this.#summary;
    const header = "runId,bench,audio,anchor,startedAt,durationS,txKbps,loss,owdP95,rttP95,stalls,verdict";
    const row = [
      this.id.runId,
      this.params.bench,
      this.params.audio,
      this.params.anchor,
      this.id.startedAt,
      s?.durationS ?? "",
      s?.txKbps.mean ?? "",
      s?.loss ?? "",
      s?.owdMs?.p95 ?? "",
      s?.rttMs?.p95 ?? "",
      s?.stalls ?? "",
      s?.verdict ?? "",
    ].join(",");
    return `${header}\n${row}\n`;
  }

  info(): LabRunInfo {
    return {
      runId: this.id.runId,
      bench: this.params.bench,
      audio: this.params.audio,
      anchor: this.params.anchor,
      startedAt: this.id.startedAt,
      durationS: this.#summary?.durationS ?? null,
      verdict: this.#summary?.verdict ?? null,
      files: this.files,
    };
  }

  #push(line: Record<string, unknown>): void {
    this.#lines.push(JSON.stringify(line));
  }
}

/**
 * Verdict of the fake run, approximating the §12.2 go/no-go rules (the real thresholds
 * are WP-05's `tools/lab-analyze/thresholds.json`): too little data or a dropped
 * anchor is inconclusive (runs whose anchor dropped are excluded, not counted as loss).
 */
export function verdictOf(p: ResolvedLabParams, seconds: readonly RunSecond[], s: LabSummary): LabVerdict {
  const linked = seconds.filter((x) => x.link !== null);
  if (linked.length < 2) return "inconclusive";
  if (seconds.some((x) => !x.anchorOk)) return "inconclusive";
  switch (p.bench) {
    case "T0":
    case "T0c":
      return s.loss < 0.05 && (s.owdMs?.p95 ?? Infinity) < 60 ? "go" : "noGo";
    case "T0b":
      return s.loss < 0.05 && (s.owdMs?.p95 ?? Infinity) < 80 ? "go" : "noGo";
    case "T1": {
      const good = linked.filter((x) => (x.link as LinkSecond).loss < 0.05).length;
      return good / seconds.length >= 0.95 ? "go" : "noGo";
    }
    default:
      return s.loss < 0.05 ? "go" : "noGo";
  }
}
