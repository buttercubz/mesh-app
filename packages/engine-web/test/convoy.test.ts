// The acceptance flow of WP-13 in Node: a simulated convoy driven through its JSON
// command surface — join the bots' ride, talk, and get busy as the third talker — plus
// the 20 ms driver and the Lab files of a bench run against a simulated peer.
import { describe, expect, it } from "vitest";

import type { Timers } from "../src/index.js";
import { MAX_STEP_MS } from "../src/index.js";
import { LINE3, bots, convoy, ok, runUntil, talkers } from "./helpers.js";

describe("SimConvoy", () => {
  it("joins the bots' ride, talks, and is busy as the third talker", async () => {
    const { c, events } = await convoy({ scenario: LINE3 });
    const url = ok(c, { type: "sim.invite" })["url"] as string;
    expect(url).toMatch(/^convlinkvoice:/);
    const gid = ok(c, { type: "joinGroup", url, displayName: "Me" })["groupId"] as string;
    ok(c, { type: "sim.startEngine", groupId: gid });
    expect(runUntil(c, 20_000, (s) => s.mesh.reachable === 3 && s.session?.groupId === gid)).toBe(true);
    ok(c, { type: "goLive" });
    ok(c, { type: "setTalkMode", mode: "ptt" });
    ok(c, { type: "rideControl", op: "start" });
    c.step(500);
    const [bot1, bot2] = bots(c).map((b) => b.nodeId as number);
    const me = c.snapshot?.session?.myNodeId as number;

    // Alone on the floor: talking, and listed as a talker.
    ok(c, { type: "sim.ptt", down: true });
    expect(runUntil(c, 1_000, (s) => s.session?.talk === "talking")).toBe(true);
    expect(talkers(c)).toContain(me);
    ok(c, { type: "sim.ptt", down: false });
    expect(runUntil(c, 1_500, (s) => s.session?.talk === "idle" && s.talkers.length === 0)).toBe(true);

    // Two bots take the floor; the page hears both.
    ok(c, { type: "sim.botTalk", bot: 1, ms: 6_000 });
    c.step(400);
    ok(c, { type: "sim.botTalk", bot: 2, ms: 6_000 });
    expect(runUntil(c, 1_500, (s) => s.talkers.length === 2)).toBe(true);
    expect(talkers(c)).toEqual([bot1, bot2].sort((a, b) => a - b));

    // Third talker: blocked with talkDenied{cap}, never listed.
    const denied = events.filter((e) => e.type === "talkDenied").length;
    ok(c, { type: "sim.ptt", down: true });
    expect(runUntil(c, 500, (s) => s.session?.talk === "blocked")).toBe(true);
    const deniedNow = events.filter((e) => e.type === "talkDenied");
    expect(deniedNow.length).toBeGreaterThan(denied);
    expect(deniedNow.at(-1)).toMatchObject({ type: "talkDenied", reason: "cap" });
    expect(talkers(c)).not.toContain(me);

    // A slot frees while PTT is held: the page gets the floor.
    expect(runUntil(c, 8_000, (s) => s.session?.talk === "talking")).toBe(true);
    ok(c, { type: "sim.ptt", down: false });
    expect(runUntil(c, 1_500, (s) => s.session?.talk === "idle")).toBe(true);
    c.dispose();
  });

  it("advances with the page clock every 20 ms, capped per tick", async () => {
    let now = 1_000;
    let tick: (() => void) | undefined;
    let cleared = false;
    const timers: Timers = {
      setInterval: (f, ms) => {
        expect(ms).toBe(20);
        tick = f;
        return 7;
      },
      clearInterval: (h) => {
        expect(h).toBe(7);
        cleared = true;
      },
    };
    const { c } = await convoy({ query: "?sim=pair&nodes=2&botsTalk=0", now: () => now, timers });
    const simNow = () => ok(c, { type: "sim.info" })["nowUs"] as number;
    const t0 = simNow();
    c.start();
    expect(c.running).toBe(true);
    for (let i = 0; i < 50; i++) {
      now += 20;
      tick?.();
    }
    expect(simNow() - t0).toBe(1_000_000);
    // A throttled tab resumes: one tick never advances more than MAX_STEP_MS.
    now += 60_000;
    tick?.();
    expect(simNow() - t0).toBe(1_000_000 + MAX_STEP_MS * 1_000);
    c.stop();
    expect(cleared && !c.running).toBe(true);
    c.dispose();
    expect(() => c.command('{"type":"sim.info"}')).toThrow(/disposed/);
  });

  it("delivers events to every listener and survives a failing one", async () => {
    const errors: unknown[] = [];
    const { c } = await convoy({ scenario: { preset: "pair", nodes: 2 }, onError: (e) => errors.push(e) });
    let a = 0;
    let b = 0;
    const offA = c.onEvent(() => {
      a++;
      throw new Error("listener bug");
    });
    c.onEvent(() => b++);
    ok(c, { type: "sim.startEngine", demo: true });
    c.step(2_000);
    expect(a).toBeGreaterThan(0);
    expect(b).toBe(a);
    expect(errors.length).toBe(a);
    offA();
    c.step(2_000);
    expect(b).toBeGreaterThan(a);
    expect(c.snapshot?.session).not.toBeNull();
    c.dispose();
  });

  it("collects the Lab files of a bench run against a simulated peer", async () => {
    const { c, events } = await convoy({ scenario: { preset: "pair", nodes: 2, botsTalk: false, seed: 5 } });
    ok(c, { type: "sim.startEngine", lab: { code: "4321" } });
    expect(runUntil(c, 20_000, (s) => s.mesh.links >= 1)).toBe(true);
    const runId = ok(c, {
      type: "labStart",
      bench: "T0",
      params: { audio: "off", anchor: "audio", gps: false, durationS: 5 },
    })["runId"] as string;
    c.step(8_000);
    const fin = events.find((e) => e.type === "labFinished");
    expect(fin).toBeDefined();
    if (fin?.type !== "labFinished") return;
    expect(fin.runId).toBe(runId);
    const files = c.labFiles();
    expect(files.map((f) => f.name)).toEqual([`${runId}.jsonl`, `${runId}-summary.csv`]);
    for (const path of fin.files) expect(c.readFile(path)).toBeDefined();
    const jsonl = c.readFile(`${runId}.jsonl`)?.data ?? "";
    const lines = jsonl.trim().split("\n").map((l) => JSON.parse(l) as { t: string; runId?: string });
    expect(lines[0]).toMatchObject({ t: "run", runId });
    expect(lines.some((l) => l.t === "summary")).toBe(true);
    expect(c.readFile(`${runId}-summary.csv`)?.data.trim().split("\n")).toHaveLength(2);
    expect(c.readFile("nope.jsonl")).toBeUndefined();
    c.forgetRun(runId);
    expect(c.labFiles()).toEqual([]);
    c.dispose();
  });
});
