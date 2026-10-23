// The web implementation mirrors the native gating (DESIGN §3.2, §11.1-11.2).
import { createEngineClient, GESTURE_COMMANDS } from "@meshapp/engine-api";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { isIntercomError } from "../src/errors.js";
import { harness, labParams } from "./helpers.js";

beforeEach(() => {
  vi.useFakeTimers({ now: Date.UTC(2026, 9, 23, 9, 12, 3) });
});

afterEach(() => {
  vi.useRealTimers();
});

const STAR_COMMANDS = [
  { type: "goLive" },
  { type: "resumeMic" },
  { type: "labStart", bench: "T0", params: labParams("full") },
  { type: "labStart", bench: "T1", params: labParams("renderOnly") },
  { type: "audioCheck", op: "start", mode: "tone" },
  { type: "audioCheck", op: "start", mode: "echo" },
  { type: "audioCheck", op: "start", mode: "voxMeter" },
] as const;

async function rejection(p: Promise<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (e) {
    return e;
  }
  throw new Error("expected a rejection");
}

describe("send", () => {
  it.each(STAR_COMMANDS)("rejects ★ %o with useGestureCommand", async (command) => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    const e = await rejection(h.plugin.send({ command: JSON.stringify(command) }));
    expect(isIntercomError(e)).toBe(true);
    expect(e).toMatchObject({ code: "useGestureCommand", message: "useGestureCommand" });
  });

  it("rejects a ★ command even with extra or malformed fields", async () => {
    const h = await harness();
    const e = await rejection(h.plugin.send({ command: JSON.stringify({ type: "goLive", extra: 1 }) }));
    expect(e).toMatchObject({ code: "useGestureCommand" });
  });

  it("covers every GESTURE_COMMANDS rule of WP-03", () => {
    const types = new Set(STAR_COMMANDS.map((c) => c.type));
    for (const rule of GESTURE_COMMANDS) expect(types.has(rule.type)).toBe(true);
  });

  it("accepts the non-★ forms of gated command types", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    expect(await h.send({ type: "labStart", bench: "T0", params: labParams("off") })).toMatchObject({ ok: true });
    expect(await h.send({ type: "labStop" })).toEqual({ ok: true });
    expect(await h.send({ type: "audioCheck", op: "stop" })).toEqual({ ok: true });
  });

  it("answers invalidCommand for malformed JSON or unknown types", async () => {
    const h = await harness();
    for (const command of ["{not json", "[]", "42", JSON.stringify({ type: "selfDestruct" })]) {
      const { result } = await h.plugin.send({ command });
      expect(JSON.parse(result)).toEqual({ ok: false, code: "invalidCommand" });
    }
    // LabParams without the mandatory audio/anchor is a contract violation.
    expect(await h.send({ type: "labStart", bench: "T0", params: { gps: false } })).toEqual({
      ok: false,
      code: "invalidCommand",
    });
  });
});

describe("gestureCommand", () => {
  it.each(STAR_COMMANDS.slice(2, 3))("accepts %o while the page is visible", async (command) => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    expect(await h.gesture(command)).toMatchObject({ ok: true });
  });

  it.each(STAR_COMMANDS)("rejects %o with notForeground while hidden", async (command) => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    h.env.setVisibility("hidden");
    const e = await rejection(h.plugin.gestureCommand({ command: JSON.stringify(command) }));
    expect(e).toMatchObject({ code: "notForeground" });
  });

  it("forwards goLive/resumeMic to the engine (no session in Lab mode)", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    expect(await h.gesture({ type: "goLive" })).toEqual({ ok: false, code: "noSession" });
    await expect(h.plugin.goLive()).rejects.toMatchObject({ code: "noSession" });
    await expect(h.plugin.resumeMic()).rejects.toMatchObject({ code: "noSession" });
  });

  it("goLive sugar is gated like gestureCommand", async () => {
    const h = await harness();
    h.env.setVisibility("hidden");
    await expect(h.plugin.goLive()).rejects.toMatchObject({ code: "notForeground" });
  });

  it("answers engineNotRunning before startEngine", async () => {
    const h = await harness();
    expect(await h.gesture({ type: "audioCheck", op: "start", mode: "tone" })).toEqual({
      ok: false,
      code: "engineNotRunning",
    });
  });
});

describe("with createEngineClient (engine-api)", () => {
  it("routes ★ commands through gestureCommand and maps refusals to codes", async () => {
    const h = await harness();
    const client = createEngineClient(h.plugin);
    const gesture = vi.spyOn(h.plugin, "gestureCommand");
    const send = vi.spyOn(h.plugin, "send");
    await h.plugin.startEngine({ lab: { code: "4711" } });

    const started = await client.command({ type: "labStart", bench: "T0", params: labParams("full") });
    expect(started).toMatchObject({ ok: true, runId: expect.stringMatching(/^r-\d{8}-\d{4}-[0-9a-f]{4}$/) });
    expect(gesture).toHaveBeenCalledTimes(1);
    expect(send).not.toHaveBeenCalled();

    expect(await client.command({ type: "labListRuns" })).toMatchObject({ ok: true });
    expect(send).toHaveBeenCalledTimes(1);

    h.env.setVisibility("hidden");
    expect(await client.command({ type: "goLive" })).toEqual({ ok: false, code: "notForeground" });
  });

  it("delivers contract-valid events to onEvent", async () => {
    const h = await harness();
    const client = createEngineClient(h.plugin);
    const types: string[] = [];
    const off = await client.onEvent((e) => types.push(e.type));
    await h.plugin.startEngine({ lab: { code: "4711" } });
    await vi.advanceTimersByTimeAsync(2000);
    await off();
    expect(types).toContain("snapshot");
    expect(types).toContain("notice");
  });
});
