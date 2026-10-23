// WebAudio playback against a fake AudioContext: one voice per remote talker, none for
// this phone, voices stop when talkers do.
import type { Snapshot, Talker } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import type { AudioContextLike } from "../src/index.js";
import { TalkerNoisePlayer, audibleTalkers } from "../src/index.js";

interface FakeParam {
  value: number;
  setTargetAtTime(v: number, t: number, k: number): void;
}

function param(): FakeParam {
  return {
    value: 0,
    setTargetAtTime(v) {
      this.value = v;
    },
  };
}

function fakeContext(): { ctx: AudioContextLike; started: number; stopped: number; resumed: number } {
  const state = { started: 0, stopped: 0, resumed: 0 };
  const node = () => ({ connect: () => undefined });
  const ctx = {
    currentTime: 1,
    sampleRate: 8_000,
    state: "suspended",
    destination: {},
    resume: async () => {
      state.resumed++;
    },
    close: async () => undefined,
    createBuffer: (_c: number, length: number) => {
      const data = new Float32Array(length);
      return { getChannelData: () => data, length };
    },
    createBufferSource: () => ({
      ...node(),
      buffer: null,
      loop: false,
      start: () => {
        state.started++;
      },
      stop: () => {
        state.stopped++;
      },
    }),
    createBiquadFilter: () => ({ ...node(), type: "", frequency: param(), Q: param() }),
    createGain: () => ({ ...node(), gain: param() }),
  };
  return {
    ctx: ctx as unknown as AudioContextLike,
    get started() {
      return state.started;
    },
    get stopped() {
      return state.stopped;
    },
    get resumed() {
      return state.resumed;
    },
  };
}

function snap(me: number, talkers: number[]): Snapshot {
  const t = (nodeId: number): Talker => ({ nodeId, name: `n${nodeId}`, hops: 1, level: 0.5, sinceMs: 0 });
  return { session: { myNodeId: me }, talkers: talkers.map(t) } as unknown as Snapshot;
}

describe("TalkerNoisePlayer", () => {
  it("plays one voice per remote talker and follows the snapshots", async () => {
    const f = fakeContext();
    const p = new TalkerNoisePlayer(f.ctx);
    await p.resume();
    expect(f.resumed).toBe(1);
    expect(audibleTalkers(snap(1, [1, 2])).map((t) => t.nodeId)).toEqual([2]);
    p.update(snap(1, [1, 2]));
    expect(p.playing).toEqual([2]);
    p.update(snap(1, [2, 3]));
    expect(p.playing).toEqual([2, 3]);
    expect(f.started).toBe(2);
    p.update(snap(1, [3]));
    expect(p.playing).toEqual([3]);
    expect(f.stopped).toBe(1);
    p.silence();
    expect(p.playing).toEqual([]);
    await p.close();
    p.update(snap(1, [4]));
    expect(p.playing).toEqual([]);
  });

  it("is unavailable without WebAudio", () => {
    expect(TalkerNoisePlayer.create()).toBeUndefined();
  });
});
