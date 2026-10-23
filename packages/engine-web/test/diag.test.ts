// The simulator's ridelog/1 recorder: a ride through the backend produces a session file
// that validates against crates/mesh-lab/schema/ridelog-1.schema.json and that
// `lab-analyze ride` reads (end to end without phones); ride keys match the native
// runtime's; recording off keeps only a RAM ring; nothing touches web storage.
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { RideLogRecorder, createWebEngineBackend, groupRideKey, labRideKey, sessionIdFor } from "../src/index.js";
import { ok, runUntil, wasmModule } from "./helpers.js";

type Validate = (value: unknown, schema: unknown) => { path: string; message: string }[];
interface AnalyzerModules {
  validate: Validate;
  loadRideSchema: () => unknown;
  readRideSources: (sources: { name: string; text: string }[], schema: unknown) => { sessions: unknown[]; problems: string[] };
  analyzeRides: (sessions: unknown[], opts: object) => { rideKey: string; timeline: { what: string }[] }[];
}

/** The analyzer's own reader and validator (tools/lab-analyze, plain ESM). */
async function analyzer(): Promise<AnalyzerModules> {
  const base = new URL("../../../tools/lab-analyze/lib/", import.meta.url);
  const schemaPath = fileURLToPath(new URL("schema.mjs", base));
  const readPath = fileURLToPath(new URL("ride/read.mjs", base));
  const indexPath = fileURLToPath(new URL("ride/index.mjs", base));
  const schema = (await import(/* @vite-ignore */ schemaPath)) as Pick<AnalyzerModules, "validate" | "loadRideSchema">;
  const read = (await import(/* @vite-ignore */ readPath)) as Pick<AnalyzerModules, "readRideSources">;
  const index = (await import(/* @vite-ignore */ indexPath)) as Pick<AnalyzerModules, "analyzeRides">;
  return { ...schema, ...read, ...index };
}

describe("ridelog/1 in the web simulator", () => {
  it("records a ride that validates and that the analyzer reads", async () => {
    let wall = Date.UTC(2026, 9, 22, 20, 4, 0);
    const b = createWebEngineBackend({
      module: await wasmModule(),
      scenario: { preset: "cluster15", nodes: 3, botsTalk: false, seed: 5 },
      wallNow: () => wall,
      appVersion: "0.1.0 (12)",
    });
    const c = await b.ready();
    c.stop();
    ok(c, { type: "setDisplayName", name: "Me" });
    const created = ok(c, { type: "createGroup", name: "Diag ride", vehicle: "moto" });
    const gid = String(created["groupId"]);
    await b.start({ groupId: gid });
    c.stop();
    b.setUiAttached(true);
    for (let i = 0; i < 12; i++) {
      wall += 1000;
      runUntil(c, 1000, () => false);
      ok(c, { type: "requestSnapshot" });
    }
    expect(b.diag.note("cmd", { name: "goLive", gesture: true, ok: true, ms: 12 })).toBe(true);
    expect(b.diag.note("Bad Name", {})).toBe(false);
    expect(b.diag.mark("crackle")).toBe(true);
    expect(b.diag.mark("free text")).toBe(false);
    await b.stop();

    const files = b.diag.files("last");
    expect(files).toHaveLength(1);
    const text = files[0]?.data ?? "";
    const lines = text.trim().split("\n").map((l) => JSON.parse(l) as Record<string, unknown>);
    expect(lines[0]).toMatchObject({ t: "hdr", schema: "ridelog/1", rideKey: await groupRideKey(gid, wall) });
    expect(lines.at(-1)).toMatchObject({ t: "end", reason: "sessionEnd" });
    expect(lines.filter((l) => l["t"] === "s").length).toBeGreaterThanOrEqual(5);
    expect(lines.some((l) => l["ev"] === "session" && typeof l["node"] === "number")).toBe(true);

    const a = await analyzer();
    const schema = a.loadRideSchema();
    const bad = lines.flatMap((l) => a.validate(l, schema).map((e) => `${String(l["t"])}${e.path} ${e.message}`));
    expect(bad).toEqual([]);
    const { sessions, problems } = a.readRideSources([{ name: files[0]?.name ?? "x", text }], schema);
    expect(problems).toEqual([]);
    const [ride] = a.analyzeRides(sessions, { all: true });
    expect(ride?.rideKey).toBe(lines[0]?.["rideKey"]);
    expect(ride?.timeline.some((t) => t.what === "MARK")).toBe(true);
    await b.dispose();
  });

  it("computes the native ride keys and session ids", async () => {
    const wall = Date.UTC(2026, 9, 22, 20, 4, 11);
    expect(sessionIdFor(wall, () => 0)).toBe("d-20261022-2004-0000");
    // Same vectors as crates/mesh-lab/tests/ridelog_schema.rs (Rust ride_key / lab_tag).
    expect(await groupRideKey("01".repeat(16), wall)).toBe("b212d3d5");
    expect(await groupRideKey("not hex", wall)).toBeNull();
    // mesh-wire lab_tag(1234) is the same SHA-256 prefix (little-endian u16).
    expect(await labRideKey(1234)).toBe("lab-bbb2");
  });

  it("keeps a RAM ring only while recording is off, never web storage", () => {
    const writes: string[] = [];
    // Swapped by descriptor: reading Node's own `localStorage` getter prints a warning.
    const before = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: new Proxy({}, { get: (_t, k) => (writes.push(String(k)), () => undefined) }),
    });
    try {
      let now = 0;
      const r = new RideLogRecorder({ model: "web-sim", os: "browser", app: "0.0.0-web" }, () => now);
      r.setEnabled(false);
      r.begin("group", null);
      for (let i = 0; i < 5000; i++) {
        now += 1;
        r.note("pad", { i });
      }
      const [f] = r.files("all");
      const lines = (f?.data ?? "").trim().split("\n");
      expect(lines.length).toBeLessThanOrEqual(4096);
      expect(lines[0]).toContain('"t":"hdr"');
      expect(r.status()).toMatchObject({ enabled: false, bytes: 0, sessions: 0 });
      expect(r.note("secret", { k: "deadbeefdeadbeefdeadbeefdeadbeef" })).toBe(true);
      expect(r.files("last")[0]?.data).toContain("<redacted>");
    } finally {
      if (before === undefined) Reflect.deleteProperty(globalThis, "localStorage");
      else Object.defineProperty(globalThis, "localStorage", before);
    }
    expect(writes).toEqual([]);
  });
});
