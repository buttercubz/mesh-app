// audioCheck through gestureCommand: 20 Hz audioCheckSample events (DESIGN §3.7).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { harness } from "./helpers.js";

beforeEach(() => {
  vi.useFakeTimers({ now: Date.UTC(2026, 9, 23, 9, 12, 3) });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("audioCheck", () => {
  it("voxMeter emits meter samples at 20 Hz until stopped", async () => {
    const h = await harness();
    await h.plugin.startEngine({});
    expect(await h.gesture({ type: "audioCheck", op: "start", mode: "voxMeter" })).toEqual({ ok: true });
    await vi.advanceTimersByTimeAsync(1000);
    const samples = h.of("audioCheckSample");
    expect(samples.length).toBeGreaterThanOrEqual(19);
    expect(samples.length).toBeLessThanOrEqual(21);
    expect(samples.every((s) => s.phase === "meter" && s.inputRate === 48_000)).toBe(true);
    expect(samples.some((s) => s.gate === "open")).toBe(true);
    expect(samples.every((s) => s.level >= 0 && s.level <= 1 && s.vadP >= 0 && s.vadP <= 1)).toBe(true);
    expect(h.of("snapshot").at(-1)?.s.audio).toMatchObject({ captureOk: true, renderOk: true });

    expect(await h.send({ type: "audioCheck", op: "stop" })).toEqual({ ok: true });
    const count = h.of("audioCheckSample").length;
    await vi.advanceTimersByTimeAsync(1000);
    expect(h.of("audioCheckSample").length).toBe(count);
    expect(h.of("snapshot").at(-1)?.s.audio).toMatchObject({ route: "none", captureOk: false });
  });

  it("echo alternates record and echo phases; tone does not capture", async () => {
    const h = await harness();
    await h.plugin.startEngine({});
    await h.gesture({ type: "audioCheck", op: "start", mode: "echo" });
    await vi.advanceTimersByTimeAsync(6500);
    const phases = h.of("audioCheckSample").map((s) => s.phase);
    expect(phases[0]).toBe("record");
    expect(phases).toContain("echo");
    expect(phases.at(-1)).toBe("record");
    expect(h.of("audioCheckSample").filter((s) => s.phase === "echo").every((s) => s.level === 0)).toBe(true);

    await h.gesture({ type: "audioCheck", op: "start", mode: "tone" });
    await vi.advanceTimersByTimeAsync(500);
    const tone = h.of("audioCheckSample").filter((s) => s.phase === "tone");
    expect(tone.length).toBeGreaterThan(0);
    expect(tone.every((s) => s.level === 0 && s.gate === "closed")).toBe(true);
  });
});
