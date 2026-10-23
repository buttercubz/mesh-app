// Lab run lifecycle of the fake engine: labStart/labStop/labMark/labListRuns/
// labDeleteRun, the 1 Hz recorder, `labProgress` at 2 Hz and `labFinished` with the
// written files (DESIGN §12.1, §12.5). Time comes from the engine tick, so a hidden tab
// whose timers were throttled catches up second by second.
import type { CommandResult, LabBench, LabParams, UiEvent } from "@meshapp/engine-api";

import type { BackendFile } from "../backend.js";
import { invalidLabParam, isPhase0Bench, resolveLabParams } from "./lab-params.js";
import { LabRun } from "./lab-run.js";
import { distanceM, gpsNear, linkSecond } from "./lab-traffic.js";
import type { Rng } from "./rng.js";

/** `labProgress` period (2 Hz). */
export const LAB_PROGRESS_MS = 500;

/** Fake GPS origins (local phone and the fake peer ≈ 12 m apart). */
const ORIGIN = { lat: 18.4861, lon: -69.9312 } as const;
const PEER_ORIGIN = { lat: 18.4862, lon: -69.9311 } as const;

export interface LabControllerDeps {
  emit(event: UiEvent): void;
  rng: Rng;
  appVersion: string;
  /** Lab node id of the fake peer when it is linked, else `null`. */
  peerLabNode(): number | null;
  uiAttached(): boolean;
}

interface ActiveRun {
  run: LabRun;
  startMs: number;
  /** Next 1 Hz sample index (1-based seconds since start). */
  nextSecond: number;
  lastProgressMs: number;
  anchorLostNotified: boolean;
}

const fail = (code: "labRunActive" | "noLabRun" | "unknownLabRun" | "invalidParams" | "notSupported") =>
  ({ ok: false, code }) as const satisfies CommandResult;

function runIdAt(ms: number, rng: Rng): string {
  const d = new Date(ms);
  const p2 = (n: number): string => String(n).padStart(2, "0");
  const date = `${d.getUTCFullYear()}${p2(d.getUTCMonth() + 1)}${p2(d.getUTCDate())}`;
  return `r-${date}-${p2(d.getUTCHours())}${p2(d.getUTCMinutes())}-${rng.hex(4)}`;
}

export class LabController {
  readonly #deps: LabControllerDeps;
  #active: ActiveRun | null = null;
  /** Stored runs, newest first. */
  #runs: LabRun[] = [];
  readonly #files = new Map<string, BackendFile>();

  constructor(deps: LabControllerDeps) {
    this.#deps = deps;
  }

  /** Audio path of the active run, or `null`. */
  get activeAudio(): LabParams["audio"] | null {
    return this.#active?.run.params.audio ?? null;
  }

  start(bench: LabBench, params: LabParams, now: number): CommandResult {
    if (this.#active !== null) return fail("labRunActive");
    if (!isPhase0Bench(bench)) return fail("notSupported");
    if (invalidLabParam(params) !== undefined) return fail("invalidParams");
    const rng = this.#deps.rng;
    const run = new LabRun(
      {
        runId: runIdAt(now, rng),
        startedAt: new Date(now).toISOString(),
        labNode: rng.int(1, 0xfffe),
        peerLabNode: this.#deps.peerLabNode(),
        appVersion: this.#deps.appVersion,
      },
      resolveLabParams(bench, params),
    );
    this.#active = { run, startMs: now, nextSecond: 1, lastProgressMs: now, anchorLostNotified: false };
    this.#runs.unshift(run);
    return { ok: true, runId: run.id.runId };
  }

  /** `labStop`: ends the active run normally (summary + `labFinished`). */
  stop(now: number): CommandResult {
    const active = this.#active;
    if (active === null) return fail("noLabRun");
    this.tick(now);
    if (this.#active !== null) this.#finish(active, (now - active.startMs) / 1000);
    return { ok: true };
  }

  /** Engine stop: the active run is cut short and stays without a summary. */
  abort(now: number): void {
    const active = this.#active;
    if (active === null) return;
    active.run.event((now - active.startMs) / 1000, "aborted", { reason: "engineStopped" });
    this.#store(active.run);
    this.#active = null;
  }

  mark(label: string, now: number): CommandResult {
    const active = this.#active;
    if (active === null) return fail("noLabRun");
    if (new TextEncoder().encode(label).length > 64) return fail("invalidParams");
    active.run.mark((now - active.startMs) / 1000, label);
    return { ok: true };
  }

  list(): CommandResult {
    return { ok: true, runs: this.#runs.map((r) => r.info()) };
  }

  delete(runId: string): CommandResult {
    if (this.#active?.run.id.runId === runId) return fail("labRunActive");
    const run = this.#runs.find((r) => r.id.runId === runId);
    if (run === undefined) return fail("unknownLabRun");
    this.#runs = this.#runs.filter((r) => r !== run);
    for (const path of run.files) this.#files.delete(path);
    return { ok: true };
  }

  readFile(path: string): BackendFile | undefined {
    return this.#files.get(path);
  }

  /** The fake peer's link went up or down. */
  peerChanged(up: boolean, labNode: number, now: number): void {
    const active = this.#active;
    if (active === null) return;
    active.run.event((now - active.startMs) / 1000, up ? "linkUp" : "linkDown", {
      link: 1,
      labNode,
      ...(up ? {} : { reason: "timeout" }),
    });
  }

  /** Advances the active run to `now`: 1 Hz samples, 2 Hz progress, finish. */
  tick(now: number): void {
    const active = this.#active;
    if (active === null) return;
    const { run } = active;
    const durationS = run.params.durationS;
    const elapsedS = (now - active.startMs) / 1000;
    while (active.nextSecond <= Math.min(elapsedS, durationS)) {
      this.#recordSecond(active, active.nextSecond);
      active.nextSecond += 1;
    }
    if (elapsedS >= durationS) {
      this.#finish(active, durationS);
      return;
    }
    if (now - active.lastProgressMs >= LAB_PROGRESS_MS) {
      active.lastProgressMs = now;
      this.#deps.emit({
        type: "labProgress",
        runId: run.id.runId,
        elapsedS: Math.round(elapsedS * 10) / 10,
        sample: run.liveSample(this.#deps.peerLabNode() === null ? 0 : 1),
      });
    }
  }

  #recordSecond(active: ActiveRun, ts: number): void {
    const { run } = active;
    const p = run.params;
    const rng = this.#deps.rng;
    const peer = this.#deps.peerLabNode();
    const attached = this.#deps.uiAttached();
    // "none" is the suspension control: in a hidden tab the anchor is gone.
    const anchorOk = p.anchor !== "none" || attached;
    const gps = p.gps ? gpsNear(rng, ORIGIN) : null;
    const peerGps = p.gps && peer !== null ? gpsNear(rng, PEER_ORIGIN) : null;
    run.addSecond({
      ts,
      link: peer === null ? null : linkSecond(rng, p),
      gps,
      peerGps: peerGps === null ? null : { lat: peerGps.lat, lon: peerGps.lon },
      distM: gps !== null && peerGps !== null ? distanceM(gps, peerGps) : null,
      anchorOk,
      background: !attached,
      peers: peer === null ? 0 : 1,
    });
    if (!anchorOk && !active.anchorLostNotified) {
      active.anchorLostNotified = true;
      run.event(ts, "anchorLost", { anchor: p.anchor });
      this.#deps.emit({ type: "notice", key: "labAnchorLost", params: { runId: run.id.runId } });
    }
  }

  #finish(active: ActiveRun, durationS: number): void {
    const summary = active.run.finish(durationS);
    this.#store(active.run);
    this.#active = null;
    this.#deps.emit({
      type: "labFinished",
      runId: active.run.id.runId,
      summary,
      files: active.run.files,
    });
  }

  #store(run: LabRun): void {
    const [jsonlPath, csvPath] = run.files as [string, string];
    this.#files.set(jsonlPath, { name: `${run.id.runId}.jsonl`, data: run.jsonl(), mime: "application/x-ndjson" });
    this.#files.set(csvPath, { name: `${run.id.runId}-summary.csv`, data: run.csv(), mime: "text/csv" });
  }
}
