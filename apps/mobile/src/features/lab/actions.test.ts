// Field test actions against the recording fake plugin (tests/helpers.ts): Lab code →
// startEngine, labStart routed through gestureCommand whenever audio ≠ off (and issued
// synchronously, inside the tap), labStop/labMark/labListRuns/labDeleteRun, export
// through shareFile, and refusals surfaced as ErrorCodes.
import { readFileSync } from "node:fs";
import type { CommandResult, LabRunInfo, UiCommand } from "@meshapp/engine-api";
import { afterEach, describe, expect, it } from "vitest";

import { createAppEngine } from "../../engine/client";
import { useLabStore } from "../../store/useLabStore";
import { createFakePlugin } from "../../../tests/helpers";
import {
  deleteRun,
  exportOrder,
  exportRun,
  joinLab,
  labFileMime,
  leaveLab,
  markRun,
  refreshRuns,
  startRun,
  stopRun,
} from "./actions";
import { benchSpec } from "./catalog";
import type { BenchSpec } from "./catalog";
import { defaultDraft, withAxis } from "./config";
import { useFieldTestStore } from "./store";
import { repoFile } from "./testing";

interface ResultFixture {
  name: string;
  for: string;
  result: CommandResult;
}
const RESULTS = JSON.parse(
  readFileSync(repoFile("crates/mesh-api/fixtures/results.json"), "utf8"),
) as ResultFixture[];
const fixture = (name: string) => {
  const found = RESULTS.find((r) => r.name === name);
  if (found === undefined) throw new Error(`fixture ${name}`);
  return found.result;
};
const FIXTURE_RUNS = (fixture("labRuns") as { runs: LabRunInfo[] }).runs;

function setup() {
  const plugin = createFakePlugin();
  plugin.answer = (cmd: UiCommand) => {
    switch (cmd.type) {
      case "labStart":
        return fixture("labRun");
      case "labListRuns":
        return fixture("labRuns");
      default:
        return { ok: true };
    }
  };
  const engine = createAppEngine(plugin, {
    setStatus: () => {},
    setInfo: () => {},
    warn: () => {},
  });
  return { plugin, engine };
}

function spec(bench: Parameters<typeof benchSpec>[0]): BenchSpec {
  const s = benchSpec(bench);
  if (s === undefined) throw new Error(`no ${bench}`);
  return s;
}

afterEach(() => {
  useFieldTestStore.getState().reset();
  useLabStore.getState().reset();
});

describe("joinLab / leaveLab", () => {
  it("starts the engine in Lab mode synchronously and lists the runs", async () => {
    const { plugin, engine } = setup();
    const pending = joinLab(engine, " 4711 ");
    // Still inside the tap: the plugin was called before any await.
    expect(plugin.calls.map((c) => c.method)).toEqual(["startEngine"]);
    expect(plugin.calls[0]?.args).toEqual({ lab: { code: "4711" } });
    expect(useFieldTestStore.getState().joining).toBe(true);
    await expect(pending).resolves.toEqual({ ok: true });
    expect(useFieldTestStore.getState().code).toBe("4711");
    await new Promise((r) => setTimeout(r, 0));
    expect(useLabStore.getState().runs.map((r) => r.runId)).toEqual(FIXTURE_RUNS.map((r) => r.runId).sort().reverse());
  });

  it("refuses a code that is not 4 digits without calling the plugin", async () => {
    const { plugin, engine } = setup();
    await expect(joinLab(engine, "47a1")).resolves.toEqual({ ok: false, code: "invalidParams" });
    expect(plugin.calls).toEqual([]);
    expect(useFieldTestStore.getState().error?.code).toBe("invalidParams");
  });

  it("maps a refused startEngine to its ErrorCode", async () => {
    const { plugin, engine } = setup();
    plugin.failStart = { code: "bluetoothOff", message: "bluetoothOff" };
    await expect(joinLab(engine, "1234")).resolves.toEqual({ ok: false, code: "bluetoothOff" });
    expect(useFieldTestStore.getState().code).toBeNull();
    expect(useFieldTestStore.getState().joining).toBe(false);
  });

  it("leaving stops the engine and clears the live ring", async () => {
    const { plugin, engine } = setup();
    await joinLab(engine, "1234");
    useLabStore.getState().progress("r-1", 0.5, {
      links: 1, peers: 1, txKbps: 1, rxKbps: 1, loss: 0, rttP50Ms: null, rttP95Ms: null,
      rssi: null, distM: null, stalls: 0, anchorOk: true,
    });
    await expect(leaveLab(engine)).resolves.toEqual({ ok: true });
    expect(plugin.calls.some((c) => c.method === "stopEngine")).toBe(true);
    expect(useFieldTestStore.getState().code).toBeNull();
    expect(useLabStore.getState().activeRunId).toBeNull();
    expect(useLabStore.getState().samples).toEqual([]);
  });
});

describe("startRun", () => {
  it("sends labStart through send when audio is off (T0 headset off)", async () => {
    const { plugin, engine } = setup();
    const t0 = spec("T0");
    const outcome = await startRun(engine, t0, defaultDraft(t0));
    expect(outcome).toEqual({ ok: true, runId: "r-20261023-0912-ab12" });
    expect(plugin.sent.find((c) => c.type === "labStart")).toBeDefined();
    expect(plugin.gestured).toEqual([]);
    expect(useFieldTestStore.getState().started?.runId).toBe("r-20261023-0912-ab12");
  });

  for (const bench of ["T1", "T3", "T2", "T5a"] as const) {
    it(`sends ${bench} (audio ≠ off) through gestureCommand, synchronously`, () => {
      const { plugin, engine } = setup();
      const s = spec(bench);
      void startRun(engine, s, defaultDraft(s));
      // The ★ command left inside the tap: no await between the call and the plugin.
      expect(plugin.calls.map((c) => c.method)).toEqual(["gestureCommand"]);
      const sent = plugin.gestured[0];
      expect(sent?.type).toBe("labStart");
      if (sent?.type !== "labStart") return;
      expect(sent.bench).toBe(bench);
      expect(sent.params.audio).toBe(s.audio);
      expect(sent.params.anchor).toBe("audio");
      expect(plugin.sent).toEqual([]);
    });
  }

  it("sends a headset T0 variant through gestureCommand with audio full", () => {
    const { plugin, engine } = setup();
    const t0 = spec("T0");
    void startRun(engine, t0, withAxis(t0, defaultDraft(t0), "headset", "hfp"));
    expect(plugin.gestured[0]?.type).toBe("labStart");
  });

  it("never sends a draft without an audio path or an anchor", async () => {
    const { plugin, engine } = setup();
    const t1 = spec("T1");
    await expect(startRun(engine, t1, { ...defaultDraft(t1), anchor: null })).resolves.toEqual({
      ok: false,
      problem: "anchorMissing",
    });
    await expect(startRun(engine, t1, { ...defaultDraft(t1), audio: null })).resolves.toEqual({
      ok: false,
      problem: "audioMissing",
    });
    expect(plugin.calls).toEqual([]);
  });

  it("surfaces an engine refusal (notForeground) and keeps no started run", async () => {
    const { plugin, engine } = setup();
    plugin.answer = () => fixture("error notForeground");
    const t1 = spec("T1");
    await expect(startRun(engine, t1, defaultDraft(t1))).resolves.toEqual({ ok: false, code: "notForeground" });
    expect(useFieldTestStore.getState().started).toBeNull();
    expect(useFieldTestStore.getState().starting).toBe(false);
    expect(useFieldTestStore.getState().error?.code).toBe("notForeground");
  });
});

describe("run commands", () => {
  it("stops, marks, lists and deletes", async () => {
    const { plugin, engine } = setup();
    await expect(stopRun(engine)).resolves.toEqual({ ok: true });
    await expect(markRun(engine, " pocket ")).resolves.toEqual({ ok: true });
    await expect(refreshRuns(engine)).resolves.toHaveLength(2);
    await expect(deleteRun(engine, "r-20261023-1001-cd34")).resolves.toEqual({ ok: true });
    expect(useLabStore.getState().runs.map((r) => r.runId)).toEqual(["r-20261023-0912-ab12"]);
    expect(plugin.sent.map((c) => c.type)).toEqual(["labStop", "labMark", "labListRuns", "labDeleteRun"]);
    expect(plugin.sent[1]).toEqual({ type: "labMark", label: "pocket" });
  });

  it("refuses empty or over-long marks locally", async () => {
    const { plugin, engine } = setup();
    await expect(markRun(engine, "  ")).resolves.toEqual({ ok: false, code: "invalidParams" });
    await expect(markRun(engine, "x".repeat(65))).resolves.toEqual({ ok: false, code: "invalidParams" });
    expect(plugin.calls).toEqual([]);
  });

  it("reports a refused labStop", async () => {
    const { plugin, engine } = setup();
    plugin.answer = () => ({ ok: false, code: "noLabRun" });
    await expect(stopRun(engine)).resolves.toEqual({ ok: false, code: "noLabRun" });
    expect(useFieldTestStore.getState().stopping).toBe(false);
  });
});

describe("export", () => {
  it("shares the JSONL first, then the CSV, with their MIME types", async () => {
    const { plugin, engine } = setup();
    const files = ["lab/r-1-summary.csv", "lab/r-1-4711.jsonl"];
    expect(exportOrder(files)).toEqual(["lab/r-1-4711.jsonl", "lab/r-1-summary.csv"]);
    await expect(exportRun(engine, "r-1", files)).resolves.toEqual({ ok: true });
    expect(plugin.calls.filter((c) => c.method === "shareFile").map((c) => c.args)).toEqual([
      { path: "lab/r-1-4711.jsonl", mime: "application/x-ndjson" },
      { path: "lab/r-1-summary.csv", mime: "text/csv" },
    ]);
    expect(useFieldTestStore.getState().exporting).toEqual([]);
    expect(labFileMime("x.bin")).toBe("application/octet-stream");
  });

  it("reports a failed share and a run without files", async () => {
    const { plugin, engine } = setup();
    plugin.shareFile = async () => {
      throw Object.assign(new Error("notSupported"), { code: "notSupported" });
    };
    await expect(exportRun(engine, "r-1", ["lab/a.jsonl"])).resolves.toEqual({ ok: false, code: "notSupported" });
    await expect(exportRun(engine, "r-2", [])).resolves.toEqual({ ok: false, code: "storageFailed" });
  });
});
