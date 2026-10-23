// The full intercom flow of the browser simulator through the backend the MeshIntercom
// web implementation uses: create a group, the simulated riders join from its invite,
// lobby, go live, ride, VOX/PTT with simulated talkers, a member dropping out and
// rejoining; plus the sim invite, Lab files, status and error mapping.
import type { CommandResult, UiCommand, UiEvent } from "@meshapp/engine-api";
import { explainUiEvent } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import type { SimConvoy } from "../src/index.js";
import { EngineWebError, WebEngineBackend, createWebEngineBackend } from "../src/index.js";
import { bots, ok, runUntil, wasmModule } from "./helpers.js";

async function backend(scenario: object): Promise<{ b: WebEngineBackend; c: SimConvoy; events: UiEvent[] }> {
  const b = createWebEngineBackend({ module: await wasmModule(), scenario });
  const events: UiEvent[] = [];
  b.setEventSink((e) => {
    expect(explainUiEvent(e)).toBeUndefined();
    events.push(e);
  });
  const c = await b.ready();
  // The tests advance simulated time by hand instead of the 20 ms page clock.
  c.stop();
  return { b, c, events };
}

/** The `groupId` a createGroup/joinGroup answer carries. */
function groupIdOf(r: CommandResult): string {
  const id = (r as { groupId?: unknown }).groupId;
  if (typeof id !== "string") throw new Error(`no groupId in ${JSON.stringify(r)}`);
  return id;
}

async function cmd(b: WebEngineBackend, command: UiCommand): Promise<CommandResult> {
  const r = await b.command(command);
  if (!r.ok) throw new Error(`${command.type} refused: ${r.code}`);
  return r;
}

describe("WebEngineBackend", () => {
  it("runs the full ride: create, riders join, lobby, live, PTT/VOX, drop out and rejoin", async () => {
    const { b, c, events } = await backend({ preset: "cluster15", nodes: 4, botsTalk: false, seed: 3 });
    expect(b.engineKind).toBe("full");
    expect(b.status()).toEqual({ engine: "idle", live: false });
    await cmd(b, { type: "setDisplayName", name: "Me" });
    const gid = groupIdOf(await cmd(b, { type: "createGroup", name: "Web ride", vehicle: "moto" }));
    const listed = await cmd(b, { type: "listGroups" });
    expect(JSON.stringify(listed)).toContain(gid);

    // Lobby: the simulated riders scanned the invite and show up.
    await b.start({ groupId: gid });
    c.stop();
    expect(b.status().engine).toBe("running");
    expect(runUntil(c, 30_000, (s) => s.mesh.reachable === 4 && s.members.length === 4)).toBe(true);
    expect(c.snapshot?.session).toMatchObject({ groupId: gid, role: "leader", ride: "lobby" });

    // Go live and start the ride; the riders follow and go live too.
    await cmd(b, { type: "goLive" });
    await cmd(b, { type: "rideControl", op: "start" });
    expect(runUntil(c, 5_000, (s) => s.session?.ride === "riding" && s.session.live)).toBe(true);
    expect(b.status()).toEqual({ engine: "running", live: true });
    c.step(2_000);
    expect(bots(c).every((x) => x.live)).toBe(true);

    // PTT through the fast path and the command, as the plugin does.
    await cmd(b, { type: "setTalkMode", mode: "ptt" });
    b.pttFast(true, 123, 456);
    await b.command({ type: "ptt", down: true });
    expect(b.lastPtt).toEqual({ down: true, tUiMs: 123, receivedAtMs: 456 });
    expect(runUntil(c, 1_000, (s) => s.session?.talk === "talking")).toBe(true);
    b.pttFast(false, undefined, 789);
    await b.command({ type: "ptt", down: false });
    expect(runUntil(c, 1_500, (s) => s.session?.talk === "idle")).toBe(true);

    // VOX: simulated speech opens the gate.
    await cmd(b, { type: "setTalkMode", mode: "vox" });
    ok(c, { type: "sim.speak", on: true });
    expect(runUntil(c, 3_000, (s) => s.session?.talk === "talking")).toBe(true);
    ok(c, { type: "sim.speak", on: false });
    expect(runUntil(c, 5_000, (s) => s.session?.talk !== "talking")).toBe(true);

    // A simulated rider talks and is heard.
    const rider = bots(c)[0];
    ok(c, { type: "sim.botTalk", bot: rider?.index, ms: 2_000 });
    expect(runUntil(c, 1_500, (s) => s.talkers.some((t) => t.nodeId === rider?.nodeId))).toBe(true);
    c.step(3_000);

    // A rider drops out of range, then rides back and rejoins.
    const away = bots(c)[2];
    ok(c, { type: "sim.botAway", bot: away?.index, away: true });
    expect(runUntil(c, 30_000, (s) => s.mesh.reachable === 3)).toBe(true);
    ok(c, { type: "sim.botAway", bot: away?.index, away: false });
    expect(
      runUntil(
        c,
        40_000,
        (s) =>
          s.mesh.reachable === 4 &&
          s.members.some((m) => m.nodeId === away?.nodeId && (m.status === "direct" || m.status === "relayed")),
      ),
    ).toBe(true);

    // Stop: back to idle; a restart works.
    await b.stop();
    expect(b.status()).toEqual({ engine: "idle", live: false });
    await b.start({ groupId: gid });
    expect(b.status().engine).toBe("running");
    expect(events.some((e) => e.type === "snapshot")).toBe(true);
    await b.dispose();
  });

  it("scans the sim invite and joins the bots' ride (Join confirm path)", async () => {
    const { b, c } = await backend({ preset: "cluster15", nodes: 3, botsTalk: false, seed: 4 });
    const scan = b.scanInviteQr();
    expect(scan.cancelled).toBe(false);
    const parsed = await cmd(b, { type: "parseInvite", url: scan.url ?? "" });
    expect(JSON.stringify(parsed)).toContain("groupName");
    const joined = await cmd(b, { type: "joinGroup", url: scan.url ?? "", displayName: "Me" });
    await b.start({ groupId: groupIdOf(joined) });
    c.stop();
    expect(runUntil(c, 30_000, (s) => s.mesh.reachable === 3)).toBe(true);
    await b.dispose();
  });

  it("maps refusals to contract error codes", async () => {
    const { b } = await backend({ preset: "pair", nodes: 2 });
    await expect(b.start({ groupId: "00".repeat(16) })).rejects.toMatchObject({ code: "unknownGroup" });
    await expect(b.start({})).rejects.toBeInstanceOf(EngineWebError);
    expect(b.status().engine).toBe("idle");
    const r = await b.command({ type: "goLive" });
    expect(r.ok).toBe(false);
    expect(b.readFile("missing.jsonl")).toBeUndefined();
    await b.dispose();

    class Custom extends Error {
      readonly code: string;
      constructor(code: string) {
        super(code);
        this.code = code;
      }
    }
    const bad = new WebEngineBackend({
      bytes: new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]),
      makeError: (code) => new Custom(code),
    });
    expect(bad.scanInviteQr()).toEqual({ cancelled: true });
    await expect(bad.command({ type: "listGroups" })).rejects.toMatchObject({ code: "internal" });
    await expect(bad.start({ demo: true })).rejects.toBeInstanceOf(Custom);
  });

  it("only emits snapshots while the UI is attached", async () => {
    const { b, c, events } = await backend({ preset: "pair", nodes: 2, botsTalk: false });
    await b.start({ demo: true });
    c.stop();
    c.step(1_000);
    b.setUiAttached(false);
    c.step(500);
    const n = events.filter((e) => e.type === "snapshot").length;
    c.step(3_000);
    expect(events.filter((e) => e.type === "snapshot").length).toBe(n);
    b.setUiAttached(true);
    c.step(500);
    expect(events.filter((e) => e.type === "snapshot").length).toBeGreaterThan(n);
    await b.dispose();
  });
});
