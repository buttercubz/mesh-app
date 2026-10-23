// C6 1.1 `diag*` methods of the web implementation: over the WASM simulator they record
// every command as a `cmd` note (never its parameters), export one `.jsonl` per session
// through the share/download path, mark with fixed labels only, toggle and clear. Over
// the scripted fake (no recorder) they answer an empty, disabled state.
import { compileMeshWasm, createWebEngineBackend } from "@meshapp/engine-web";
import { describe, expect, it } from "vitest";

import { DIAG_MARK_LABELS, PLUGIN_API_VERSION } from "../src/definitions.js";
import { IntercomError } from "../src/errors.js";
import { MeshIntercomWeb } from "../src/web.js";
import { FakeEnvironment, harness } from "./helpers.js";

let module: Promise<WebAssembly.Module> | undefined;
const wasm = (): Promise<WebAssembly.Module> => (module ??= compileMeshWasm());

describe("diagnostics (C6 1.1)", () => {
  it("records commands, marks and exports one file per session", async () => {
    const env = new FakeEnvironment();
    const backend = createWebEngineBackend({
      module: await wasm(),
      scenario: { preset: "cluster15", nodes: 3, botsTalk: false, seed: 13 },
      makeError: (code, detail) => new IntercomError(code, detail),
    });
    const plugin = new MeshIntercomWeb({ backend, environment: env });
    const convoy = await backend.ready();
    await plugin.send({ command: JSON.stringify({ type: "setDisplayName", name: "Me" }) });
    const created = JSON.parse(
      (await plugin.send({ command: JSON.stringify({ type: "createGroup", name: "Diag", vehicle: "moto" }) })).result,
    ) as { groupId: string };
    await plugin.startEngine({ groupId: created.groupId });
    convoy.stop();
    await plugin.gestureCommand({ command: JSON.stringify({ type: "goLive" }) });
    await plugin.diagMark({ label: "crackle" });
    await expect(plugin.diagMark({ label: "free text" as never })).rejects.toMatchObject({ code: "invalidParams" });

    const st = await plugin.diagStatus();
    expect(st).toMatchObject({ enabled: true, sessions: 1 });
    expect(st.rideKey).toMatch(/^[0-9a-f]{8}$/);
    const out = await plugin.diagExport({ scope: "last" });
    expect(out.sessions).toBe(1);
    expect(out.path).toMatch(/^ridelog-d-\d{8}-\d{4}-[0-9a-z]{4}\.jsonl$/);
    const text = env.shared.at(-1)?.data ?? "";
    const lines = text.trim().split("\n").map((l) => JSON.parse(l) as Record<string, unknown>);
    const cmds = lines.filter((l) => l["ev"] === "cmd");
    expect(cmds.find((c) => c["name"] === "goLive")).toMatchObject({ src: "ui", gesture: true, ok: true });
    // Parameters never reach the log (the group name was "Diag").
    expect(text).not.toContain('"Diag"');
    expect(lines.some((l) => l["t"] === "mark" && l["label"] === "crackle")).toBe(true);
    expect((await plugin.diagList()).sessions).toHaveLength(1);

    await plugin.diagSetEnabled({ enabled: false });
    expect((await plugin.diagStatus()).enabled).toBe(false);
    await expect(plugin.diagSetEnabled({ enabled: "yes" as never })).rejects.toMatchObject({ code: "invalidParams" });
    await plugin.diagSetEnabled({ enabled: true });
    await plugin.stopEngine();
    await plugin.diagClear();
    expect((await plugin.diagList()).sessions).toHaveLength(0);
    await expect(plugin.diagExport({ scope: "week" as never })).rejects.toMatchObject({ code: "invalidParams" });
    await plugin.dispose();
  });

  it("answers an empty disabled state over the scripted fake", async () => {
    const h = await harness();
    expect(await h.plugin.diagStatus()).toEqual({
      enabled: false,
      sessionId: null,
      rideKey: null,
      node: null,
      bytes: 0,
      sessions: 0,
      oldestMs: null,
    });
    expect(await h.plugin.diagExport({ scope: "all" })).toEqual({ path: "", mime: "", sessions: 0 });
    expect((await h.plugin.diagList()).sessions).toEqual([]);
  });

  it("declares the C6 version and the fixed mark labels", () => {
    expect(PLUGIN_API_VERSION).toBe("1.1");
    expect(DIAG_MARK_LABELS).toEqual(["crackle", "cantHear", "notHeard", "lag", "dropped", "other"]);
  });
});
