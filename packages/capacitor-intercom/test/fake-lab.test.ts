// The Phase 0 fake Lab peer: Lab runs end to end through the web plugin (DESIGN §12).
import type { LabRunInfo } from "@meshapp/engine-api";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { harness, labParams } from "./helpers.js";

beforeEach(() => {
  vi.useFakeTimers({ now: Date.UTC(2026, 9, 23, 9, 12, 3) });
});

afterEach(() => {
  vi.useRealTimers();
});

async function labHarness(seed = "test") {
  const h = await harness(seed);
  await h.plugin.startEngine({ lab: { code: "4711" } });
  await vi.advanceTimersByTimeAsync(2000); // the fake peer joins after 1.5 s
  return h;
}

/** Contract C7 checker of WP-05 (tools/lab-analyze): `labrun/1` lines against the schema. */
interface LabAnalyzeSchema {
  loadSchema(): unknown;
  validate(value: unknown, schema: unknown): { path: string; message: string }[];
}
const labAnalyzeSchema = new URL("../../../tools/lab-analyze/lib/schema.mjs", import.meta.url).href;

async function schemaProblems(text: string): Promise<string[]> {
  const m = (await import(/* @vite-ignore */ labAnalyzeSchema)) as LabAnalyzeSchema;
  const schema = m.loadSchema();
  return jsonLines(text).flatMap((line, i) =>
    m.validate(line, schema).map((e) => `line ${i + 1} (${String(line["t"])}): ${e.path} ${e.message}`),
  );
}

const jsonLines = (text: string): Record<string, unknown>[] =>
  text
    .trimEnd()
    .split("\n")
    .map((l) => JSON.parse(l) as Record<string, unknown>);

describe("T0 run", () => {
  it("runs to completion with progress, summary and labrun/1 files", async () => {
    const h = await labHarness();
    const started = await h.gesture({
      type: "labStart",
      bench: "T0",
      params: labParams("full", { durationS: 20, gps: true, variant: "pocket" }),
    });
    expect(started.ok).toBe(true);
    const runId = (started as { runId: string }).runId;
    expect(runId).toMatch(/^r-20261023-0912-[0-9a-f]{4}$/);

    await vi.advanceTimersByTimeAsync(10_000);
    const progress = h.of("labProgress");
    // 2 Hz progress for ≈ 10 s.
    expect(progress.length).toBeGreaterThanOrEqual(19);
    expect(progress.length).toBeLessThanOrEqual(21);
    const live = progress.at(-1)!;
    expect(live.runId).toBe(runId);
    expect(live.sample).toMatchObject({ links: 1, peers: 1, anchorOk: true });
    expect(live.sample.txKbps).toBeGreaterThan(40); // 2 voice-like streams ≈ 47 kbit/s
    expect(live.sample.distM).toEqual(expect.any(Number));
    expect(h.of("snapshot").at(-1)?.s.audio).toMatchObject({ route: "builtinSpeaker", captureOk: true });

    expect(await h.send({ type: "labMark", label: "pocket" })).toEqual({ ok: true });
    await vi.advanceTimersByTimeAsync(10_500);

    const finished = h.of("labFinished");
    expect(finished).toHaveLength(1);
    const done = finished[0]!;
    expect(done.runId).toBe(runId);
    expect(done.files).toEqual([`lab/${runId}.jsonl`, `lab/${runId}-summary.csv`]);
    expect(done.summary).toMatchObject({ durationS: 20, verdict: "go", stalls: expect.any(Number) });
    expect(done.summary.loss).toBeLessThan(0.05);
    expect(done.summary.rttMs?.p95).toBeGreaterThan(done.summary.rttMs?.p50 ?? Infinity);

    const lines = jsonLines(h.engine.readFile(done.files[0]!)!.data);
    expect(lines[0]).toMatchObject({
      t: "run",
      v: 1,
      runId,
      bench: "T0",
      audio: "full",
      anchor: "audio",
      startedAt: "2026-10-23T09:12:05.000Z",
      device: { platform: "web" },
      variant: { label: "pocket", pattern: "voiceLike" },
      params: { durationS: 20, streams: 2, payloadB: 118, intervalMs: 40 },
    });
    expect(lines.filter((l) => l["t"] === "s")).toHaveLength(20);
    expect(lines.filter((l) => l["t"] === "raw")).toHaveLength(20);
    expect(lines.find((l) => l["t"] === "mark")).toMatchObject({ label: "pocket" });
    expect(lines.at(-1)).toMatchObject({ t: "summary", verdict: "go", durationS: 20 });
    expect(lines.find((l) => l["t"] === "s")).toMatchObject({
      anchor: { ok: true, session: true, location: false, liveActivity: false },
      bt: "on",
    });
    const raw = lines.find((l) => l["t"] === "raw") as { pings: number[][] };
    expect(raw.pings.length).toBeGreaterThan(0);
    for (const [, t1, t4, hold] of raw.pings) {
      expect(t4).toBeGreaterThanOrEqual(t1!);
      expect(hold).toBe(0);
    }
    // Every line validates against the labrun/1 schema (C7).
    expect(await schemaProblems(h.engine.readFile(done.files[0]!)!.data)).toEqual([]);

    const csv = h.engine.readFile(done.files[1]!)!.data.trimEnd().split("\n");
    expect(csv).toHaveLength(2);
    expect(csv[1]!.startsWith(`${runId},T0,full,audio,`)).toBe(true);
    expect(csv[1]!.endsWith(",go")).toBe(true);

    const listed = await h.send({ type: "labListRuns" });
    expect(listed).toMatchObject({ ok: true });
    const runs = (listed as { runs: LabRunInfo[] }).runs;
    expect(runs[0]).toMatchObject({ runId, bench: "T0", durationS: 20, verdict: "go" });

    await h.plugin.shareFile({ path: done.files[0]!, mime: "application/x-ndjson" });
    expect(h.env.shared[0]).toMatchObject({ name: `${runId}.jsonl`, mime: "application/x-ndjson" });
  });

  it("labStop ends the run early with a summary", async () => {
    const h = await labHarness();
    const r = await h.send({ type: "labStart", bench: "T0", params: labParams("off") });
    await vi.advanceTimersByTimeAsync(5_200);
    expect(await h.send({ type: "labStop" })).toEqual({ ok: true });
    const done = h.of("labFinished")[0]!;
    expect(done.runId).toBe((r as { runId: string }).runId);
    expect(done.summary.durationS).toBeCloseTo(5.2, 1);
    expect(h.of("snapshot").at(-1)?.s.audio.route).toBe("none");
    expect(await h.send({ type: "labStop" })).toEqual({ ok: false, code: "noLabRun" });
    expect(await h.send({ type: "labMark", label: "x" })).toEqual({ ok: false, code: "noLabRun" });
  });

  it("a run without a peer is inconclusive", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    await h.send({ type: "labStart", bench: "T0", params: labParams("off", { durationS: 1 }) });
    await vi.advanceTimersByTimeAsync(1_100);
    expect(h.of("labFinished")[0]?.summary.verdict).toBe("inconclusive");
  });
});

describe("Lab command errors", () => {
  it("refuses a second run, later-phase benches and bad parameters", async () => {
    const h = await labHarness();
    expect(await h.send({ type: "labStart", bench: "T8", params: labParams() })).toEqual({
      ok: false,
      code: "notSupported",
    });
    expect(await h.send({ type: "labStart", bench: "T0", params: labParams("off", { streams: 9 }) })).toEqual({
      ok: false,
      code: "invalidParams",
    });
    expect(await h.send({ type: "labStart", bench: "T0", params: labParams() })).toMatchObject({ ok: true });
    expect(await h.send({ type: "labStart", bench: "T1", params: labParams() })).toEqual({
      ok: false,
      code: "labRunActive",
    });
    // No audio check during a Lab run.
    expect(await h.gesture({ type: "audioCheck", op: "start", mode: "tone" })).toEqual({
      ok: false,
      code: "wrongMode",
    });
  });

  it("needs Lab mode for labStart", async () => {
    const h = await harness();
    await h.plugin.startEngine({});
    expect(await h.send({ type: "labStart", bench: "T0", params: labParams() })).toEqual({
      ok: false,
      code: "wrongMode",
    });
  });

  it("labDeleteRun removes a stored run and its files", async () => {
    const h = await labHarness();
    const r = (await h.send({ type: "labStart", bench: "T0", params: labParams("off", { durationS: 2 }) })) as {
      runId: string;
    };
    expect(await h.send({ type: "labDeleteRun", runId: r.runId })).toEqual({ ok: false, code: "labRunActive" });
    await vi.advanceTimersByTimeAsync(2_100);
    const files = h.of("labFinished")[0]!.files;
    expect(await h.send({ type: "labDeleteRun", runId: r.runId })).toEqual({ ok: true });
    expect(h.engine.readFile(files[0]!)).toBeUndefined();
    expect(await h.send({ type: "labDeleteRun", runId: r.runId })).toEqual({ ok: false, code: "unknownLabRun" });
    expect(await h.send({ type: "labListRuns" })).toEqual({ ok: true, runs: [] });
  });
});

describe("anchors and engine stop", () => {
  it("anchor none in a hidden page reports labAnchorLost and is inconclusive", async () => {
    const h = await labHarness();
    await h.send({
      type: "labStart",
      bench: "T3",
      params: { audio: "off", anchor: "none", gps: false, durationS: 4 },
    });
    await vi.advanceTimersByTimeAsync(1_100);
    h.env.setVisibility("hidden");
    await vi.advanceTimersByTimeAsync(3_100);
    expect(h.of("notice").filter((n) => n.key === "labAnchorLost")).toHaveLength(1);
    expect(h.of("labFinished")[0]?.summary.verdict).toBe("inconclusive");
  });

  it("stopEngine cuts the run short without a summary", async () => {
    const h = await labHarness();
    const r = (await h.send({ type: "labStart", bench: "T1", params: labParams() })) as { runId: string };
    await vi.advanceTimersByTimeAsync(3_000);
    await h.plugin.stopEngine();
    expect(h.of("labFinished")).toHaveLength(0);
    const runs = (await h.send({ type: "labListRuns" })) as { runs: LabRunInfo[] };
    expect(runs.runs[0]).toMatchObject({ runId: r.runId, durationS: null, verdict: null });
    const lines = jsonLines(h.engine.readFile(runs.runs[0]!.files[0]!)!.data);
    expect(lines.at(-1)).toMatchObject({ t: "e", ev: "aborted" });
  });
});

describe("determinism", () => {
  it("the same seed and Lab code give the same run", async () => {
    const run = async () => {
      const h = await labHarness("same");
      await h.send({ type: "labStart", bench: "T0", params: labParams("off", { durationS: 3 }) });
      await vi.advanceTimersByTimeAsync(3_100);
      return h.of("labFinished")[0]!.summary;
    };
    const a = await run();
    vi.setSystemTime(Date.UTC(2026, 9, 23, 9, 12, 3));
    const b = await run();
    expect(b).toEqual(a);
  });
});
