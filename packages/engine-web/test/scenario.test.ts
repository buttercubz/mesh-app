import { describe, expect, it } from "vitest";

import { scenarioFromQuery, scenarioJson } from "../src/index.js";
import { convoy, ok } from "./helpers.js";

describe("scenario query parameters", () => {
  it("reads ?sim=chain5&loss=0.1 and friends, ignoring the app's own parameters", () => {
    const { scenario, page } = scenarioFromQuery(
      "?sim=chain5&loss=0.1&seed=42&botsTalk=0&talkMeanS=5&roam=1&roamMeanS=30&shadow=0-2,1:3,x&audio=1&lang=es",
    );
    expect(scenario).toEqual({
      preset: "chain5",
      loss: 0.1,
      seed: 42,
      botsTalk: false,
      botTalkMeanS: 5,
      roam: true,
      roamMeanS: 30,
      shadow: [
        [0, 2],
        [1, 3],
      ],
    });
    expect(page.audio).toBe(true);
    expect(scenarioFromQuery("").scenario).toEqual({});
    expect(scenarioFromQuery("sim=ring9&nodes=abc&seed=-1").scenario).toEqual({});
    expect(scenarioFromQuery("?preset=pair&nodes=3").scenario).toEqual({ preset: "pair", nodes: 3 });
    expect(scenarioJson({ preset: "pair", loss: undefined })).toBe('{"preset":"pair"}');
  });

  it("builds the world the query describes", async () => {
    const { c } = await convoy({ query: "?sim=chain5&loss=0.1&seed=9" });
    expect(ok(c, { type: "sim.info" })).toMatchObject({ preset: "chain5", nodes: 5, seed: 9 });
    c.dispose();
    const d = await convoy({ query: "?sim=chain5", scenario: { nodes: 3 } });
    expect(ok(d.c, { type: "sim.info" })).toMatchObject({ preset: "chain5", nodes: 3 });
    d.c.dispose();
    await expect(convoy({ scenario: { nodes: 99 } })).rejects.toThrow(/scenario refused/);
  });

  it("is deterministic for a seed", async () => {
    const transcript = async () => {
      const { c, events } = await convoy({ scenario: { preset: "pair", nodes: 3, seed: 11, wallMs: 1 } });
      ok(c, { type: "sim.startEngine", demo: true });
      c.step(5_000);
      c.dispose();
      return JSON.stringify(events);
    };
    expect(await transcript()).toBe(await transcript());
  });
});
