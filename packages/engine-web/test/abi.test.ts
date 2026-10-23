// The module itself: exactly the §11.2 exports, no imports, the size report written by
// `cargo xtask build-wasm`, and the raw handle/framing conventions.
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ABI_EXPORTS, MeshWasm, MeshWasmError, defaultWasmUrl, loadMeshWasm } from "../src/index.js";
import { wasmModule } from "./helpers.js";

describe("mesh_wasm.wasm", () => {
  it("exports exactly the §11.2 functions plus memory and imports nothing", async () => {
    const m = await wasmModule();
    expect(WebAssembly.Module.imports(m)).toEqual([]);
    const exports = WebAssembly.Module.exports(m);
    const fns = exports.filter((e) => e.kind === "function").map((e) => e.name).sort();
    expect(fns).toEqual([...ABI_EXPORTS]);
    expect(exports.some((e) => e.kind === "memory" && e.name === "memory")).toBe(true);
  });

  it("ships the size report of xtask build-wasm", () => {
    const wasmPath = fileURLToPath(defaultWasmUrl());
    const meta = JSON.parse(readFileSync(wasmPath.replace(/\.wasm$/, ".json"), "utf8")) as {
      bytes: number;
      exports: string[];
    };
    expect(meta.bytes).toBe(statSync(wasmPath).size);
    expect(meta.exports).toEqual([...ABI_EXPORTS]);
    // DWARF is stripped: the browser download stays a few MiB.
    expect(meta.bytes).toBeLessThan(8 * 1024 * 1024);
  });

  it("loads from the package default location in Node", async () => {
    const w = await loadMeshWasm();
    expect(w).toBeInstanceOf(MeshWasm);
    const h = w.worldNew("{}");
    expect(h).toBeGreaterThan(0);
    w.worldFree(h);
  });

  it("refuses bad scenarios and unknown handles, frames every answer", async () => {
    const w = await MeshWasm.instantiate(await wasmModule());
    expect(w.worldNew('{"preset":"ring9"}')).toBe(0);
    expect(w.worldNew('{"colour":"red"}')).toBe(0);
    expect(w.worldNew("not json")).toBe(0);
    expect(w.command(42, '{"type":"sim.info"}')).toBeUndefined();
    expect(w.pollEvents(42)).toBeUndefined();
    expect(w.pollLab(42)).toBeUndefined();
    w.step(42, 20_000);
    w.worldFree(42);

    const a = w.worldNew('{"preset":"pair","nodes":2,"botsTalk":false}');
    const b = w.worldNew("");
    expect([a, b].every((h) => h > 0) && a !== b).toBe(true);
    const info = JSON.parse(w.command(a, '{"type":"sim.info"}') ?? "{}") as Record<string, unknown>;
    expect(info).toMatchObject({ ok: true, preset: "pair", nodes: 2 });
    expect(JSON.parse(w.command(a, "{oops") ?? "{}")).toEqual({ ok: false, code: "invalidCommand" });
    expect(JSON.parse(w.command(a, '{"type":"sim.nope"}') ?? "{}")).toEqual({
      ok: false,
      code: "invalidCommand",
    });
    // Non-ASCII survives the UTF-8 round trip.
    const r = JSON.parse(w.command(a, '{"type":"setDisplayName","name":"Ñandú 🏍"}') ?? "{}") as {
      ok: boolean;
    };
    expect(r.ok).toBe(true);
    w.step(a, 100_000);
    expect(Array.isArray(JSON.parse(w.pollEvents(a) ?? "null"))).toBe(true);
    expect(w.pollLab(a)).toBe("");
    w.worldFree(a);
    expect(w.command(a, '{"type":"sim.info"}')).toBeUndefined();
    // The freed handle is reused.
    expect(w.worldNew("{}")).toBe(a);
  });

  it("rejects a module with the wrong surface", async () => {
    // (module (memory (export "memory") 1))
    const bytes = new Uint8Array([
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x0a, 0x01,
      0x06, 0x6d, 0x65, 0x6d, 0x6f, 0x72, 0x79, 0x02, 0x00,
    ]);
    const m = await WebAssembly.compile(bytes);
    await expect(MeshWasm.instantiate(m)).rejects.toBeInstanceOf(MeshWasmError);
    await expect(loadMeshWasm({ bytes })).rejects.toThrow(/lacks exports/);
  });
});
