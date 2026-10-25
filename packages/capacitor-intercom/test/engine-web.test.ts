// Phase 1: the web implementation backed by @meshapp/engine-web (the real Rust core in
// mesh_wasm.wasm, simulated convoy). The plugin's gating holds over the real engine:
// `send` refuses ★ commands, `gestureCommand` needs a visible page, and a full ride
// (create, lobby, go live, ride, PTT) runs through the C6 surface.
import type { CommandResult, Snapshot, UiEvent } from "@meshapp/engine-api";
import { explainUiEvent } from "@meshapp/engine-api";
import type { SimConvoy, WebEngineBackend } from "@meshapp/engine-web";
import { compileMeshWasm, createWebEngineBackend } from "@meshapp/engine-web";
import { describe, expect, it } from "vitest";

import { IntercomError } from "../src/errors.js";
import { MeshIntercomWeb, createDefaultBackend, wantsFakeEngine } from "../src/web.js";
import { FakeEnvironment } from "./helpers.js";

let module: Promise<WebAssembly.Module> | undefined;
const wasm = (): Promise<WebAssembly.Module> => (module ??= compileMeshWasm());

interface WasmHarness {
  plugin: MeshIntercomWeb;
  env: FakeEnvironment;
  backend: WebEngineBackend;
  convoy: SimConvoy;
  events: UiEvent[];
  send(command: object): Promise<CommandResult>;
  gesture(command: object): Promise<CommandResult>;
}

async function harness(): Promise<WasmHarness> {
  const env = new FakeEnvironment();
  const backend = createWebEngineBackend({
    module: await wasm(),
    scenario: { preset: "cluster15", nodes: 3, botsTalk: false, seed: 11 },
    makeError: (code, detail) => new IntercomError(code, detail),
  });
  const plugin = new MeshIntercomWeb({ backend, environment: env });
  const events: UiEvent[] = [];
  await plugin.addListener("event", ({ json }: { json: string }) => {
    const event = JSON.parse(json) as UiEvent;
    expect(explainUiEvent(event)).toBeUndefined();
    events.push(event);
  });
  const convoy = await backend.ready();
  const parse = (r: { result: string }): CommandResult => JSON.parse(r.result) as CommandResult;
  return {
    plugin,
    env,
    backend,
    convoy,
    events,
    send: async (c) => parse(await plugin.send({ command: JSON.stringify(c) })),
    gesture: async (c) => parse(await plugin.gestureCommand({ command: JSON.stringify(c) })),
  };
}

/** Steps simulated time (the tests drive the clock instead of the 20 ms timer). */
function runUntil(c: SimConvoy, limitMs: number, pred: (s: Snapshot) => boolean): boolean {
  for (let t = 0; t <= limitMs; t += 20) {
    if (c.snapshot !== undefined && pred(c.snapshot)) return true;
    c.step(20);
  }
  return c.snapshot !== undefined && pred(c.snapshot);
}

function groupIdOf(r: CommandResult): string {
  const id = (r as { groupId?: unknown }).groupId;
  if (typeof id !== "string") throw new Error(`no groupId in ${JSON.stringify(r)}`);
  return id;
}

describe("default backend", () => {
  it("is the WASM simulator unless the page asks for the fake (?engine=fake)", async () => {
    const env = new FakeEnvironment();
    const full = createDefaultBackend(env);
    expect(full.engineKind).toBe("full");
    env.search = "?sim=pair&engine=fake";
    expect(createDefaultBackend(env).engineKind).toBe("platform-only");
    expect(wantsFakeEngine("?engine=fake")).toBe(true);
    expect(wantsFakeEngine("?sim=chain5")).toBe(false);
    expect(wantsFakeEngine("")).toBe(false);

    const plugin = new MeshIntercomWeb({ environment: new FakeEnvironment() });
    expect(await plugin.getInfo()).toMatchObject({ platform: "web", engine: "full" });
    await plugin.dispose();
    await (full as WebEngineBackend).dispose();
  });
});

describe("MeshIntercomWeb over @meshapp/engine-web", () => {
  it("runs a ride through the C6 surface with native gating", async () => {
    const h = await harness();
    expect(await h.plugin.getInfo()).toMatchObject({ platform: "web", engine: "full", apiVersion: "1.2" });
    expect(await h.plugin.getStatus()).toEqual({ engine: "idle", live: false, uiAttached: true });

    expect((await h.send({ type: "setDisplayName", name: "Me" })).ok).toBe(true);
    const gid = groupIdOf(await h.send({ type: "createGroup", name: "Web ride", vehicle: "moto" }));

    // startEngine needs a visible page, like gestureCommand.
    h.env.setVisibility("hidden");
    await expect(h.plugin.startEngine({ groupId: gid })).rejects.toMatchObject({ code: "notForeground" });
    h.env.setVisibility("visible");
    await h.plugin.startEngine({ groupId: gid });
    h.convoy.stop();
    expect((await h.plugin.getStatus()).engine).toBe("running");
    expect(runUntil(h.convoy, 30_000, (s) => s.mesh.reachable === 3)).toBe(true);

    // ★ commands: refused by send, refused by gestureCommand while hidden.
    await expect(h.plugin.send({ command: '{"type":"goLive"}' })).rejects.toMatchObject({
      code: "useGestureCommand",
    });
    await expect(h.plugin.send({ command: '{"type":"audioCheck","op":"start","kind":"tone"}' })).rejects.toThrow(
      "useGestureCommand",
    );
    h.env.setVisibility("hidden");
    await expect(h.gesture({ type: "goLive" })).rejects.toMatchObject({ code: "notForeground" });
    h.env.setVisibility("visible");
    expect(await h.gesture({ type: "goLive" })).toMatchObject({ ok: true });

    expect((await h.send({ type: "rideControl", op: "start" })).ok).toBe(true);
    expect(runUntil(h.convoy, 5_000, (s) => s.session?.ride === "riding" && s.session.live)).toBe(true);
    expect(await h.plugin.getStatus()).toEqual({ engine: "running", live: true, uiAttached: true });

    // PTT fast path + command.
    expect((await h.send({ type: "setTalkMode", mode: "ptt" })).ok).toBe(true);
    await h.plugin.ptt({ down: true, tUiMs: 42 });
    expect(h.backend.lastPtt).toMatchObject({ down: true, tUiMs: 42 });
    expect(runUntil(h.convoy, 1_000, (s) => s.session?.talk === "talking")).toBe(true);
    await h.plugin.ptt({ down: false });
    expect(runUntil(h.convoy, 1_500, (s) => s.session?.talk === "idle")).toBe(true);

    // Keep-screen-on and the notification catalog (no-op on the web).
    await h.plugin.setKeepScreenOn({ on: true });
    expect(h.env.wakeLocksRequested).toBe(1);
    await h.plugin.setNotificationCatalog({ title: "convLink Voice", templates: { "notices.x": "x" } });

    // Snapshots reached the listener; none while hidden.
    expect(h.events.some((e) => e.type === "snapshot")).toBe(true);
    h.env.setVisibility("hidden");
    h.convoy.step(500);
    const n = h.events.filter((e) => e.type === "snapshot").length;
    h.convoy.step(2_000);
    expect(h.events.filter((e) => e.type === "snapshot").length).toBe(n);
    expect(h.plugin.keepScreenOn).toBe(false);

    await h.plugin.stopEngine();
    expect(await h.plugin.getStatus()).toEqual({ engine: "idle", live: false, uiAttached: false });
    await h.plugin.dispose();
    await h.backend.dispose();
  });

  it("answers the sim invite and maps engine refusals to contract codes", async () => {
    const h = await harness();
    const scan = await h.plugin.scanInviteQr();
    expect(scan.cancelled).toBe(false);
    expect(typeof scan.url).toBe("string");
    await expect(h.plugin.startEngine({ groupId: "00".repeat(16) })).rejects.toMatchObject({
      code: "unknownGroup",
    });
    await expect(h.plugin.startEngine({ groupId: "00".repeat(16) })).rejects.toBeInstanceOf(IntercomError);
    await expect(h.plugin.ptt({ down: true })).rejects.toMatchObject({ code: "engineNotRunning" });
    expect(await h.send({ type: "nope" })).toEqual({ ok: false, code: "invalidCommand" });
    await h.plugin.dispose();
    await h.backend.dispose();
  });
});
