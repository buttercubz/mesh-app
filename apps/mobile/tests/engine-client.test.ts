// engine/client.ts (WP-18 interface): ★ commands only through gestureCommand, every
// other command through send, setPrompts on every startEngine, PTT stamping.
import type { UiCommand } from "@meshapp/engine-api";
import { PROMPT_KEYS, UI_COMMAND_TYPES, isUiCommand, requiresGesture } from "@meshapp/engine-api";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createAppEngine } from "../src/engine/client";
import { promptTemplates, setPromptsCommand } from "../src/engine/prompts";
import { createFakePlugin, flush } from "./helpers";

const labParams = (audio: "off" | "renderOnly" | "full") => ({
  bench: "T0" as const,
  params: { audio, anchor: "audio" as const, gps: false },
});

/** One valid command per UiCommand type (★ variants included). */
const SAMPLE_COMMANDS: UiCommand[] = [
  { type: "createGroup", name: "Sunday", vehicle: "moto" },
  { type: "parseInvite", url: "convlinkvoice://join?v=1#abc" },
  { type: "joinGroup", url: "convlinkvoice://join?v=1#abc", displayName: "Ana" },
  { type: "listGroups" },
  { type: "forgetGroup", groupId: "0123456789abcdef0123456789abcdef" },
  { type: "getInvite", groupId: "0123456789abcdef0123456789abcdef" },
  { type: "goLive" },
  { type: "resumeMic" },
  { type: "audioCheck", op: "start", mode: "tone" },
  { type: "audioCheck", op: "start", mode: "voxMeter" },
  { type: "audioCheck", op: "stop" },
  { type: "setTalkMode", mode: "ptt" },
  { type: "ptt", down: true },
  { type: "setPttStyle", style: "latch" },
  { type: "setVoxSensitivity", level: "high" },
  { type: "rideControl", op: "start" },
  { type: "setDisplayName", name: "Ana" },
  { type: "setVehicle", vehicle: "car" },
  { type: "setRoutePreference", pref: "headset" },
  { type: "setShareDistance", enabled: true },
  setPromptsCommand("en", true),
  { type: "setBuiltinMicTx", allowed: false },
  { type: "removeMember", nodeId: 3 },
  { type: "lockGroup" },
  { type: "closeGroup" },
  { type: "previewAccessLoss" },
  { type: "transferLeader", nodeId: 3 },
  { type: "setInviteExpiry", hours: null },
  { type: "requestSnapshot" },
  { type: "labStart", ...labParams("off") },
  { type: "labStart", ...labParams("renderOnly") },
  { type: "labStart", ...labParams("full") },
  { type: "labStop" },
  { type: "labMark", label: "bridge" },
  { type: "labListRuns" },
  { type: "labDeleteRun", runId: "r1" },
  { type: "joinOpenChannel", channel: 3, displayName: "Ana" },
  { type: "setMemberMuted", nodeId: 9, muted: true },
  { type: "setMemberBlocked", nodeId: 9, blocked: false },
];

const answerFor = (cmd: UiCommand) => {
  switch (cmd.type) {
    case "createGroup":
    case "joinGroup":
    case "joinOpenChannel":
      return { ok: true as const, groupId: "0123456789abcdef0123456789abcdef" };
    case "listGroups":
      return { ok: true as const, groups: [] };
    case "getInvite":
      return { ok: true as const, qrPng: "data:image/png;base64,AA==", hideAfterMs: 60000 };
    case "parseInvite":
      return {
        ok: true as const,
        valid: true,
        groupName: "Sunday",
        groupId: "0123456789abcdef0123456789abcdef",
        epoch: 0,
        expired: false,
        alreadyMember: false,
        leaderSigOk: true,
      };
    case "previewAccessLoss":
      return { ok: true as const, unconfirmed: [] };
    case "labStart":
      return { ok: true as const, runId: "r1" };
    case "labListRuns":
      return { ok: true as const, runs: [] };
    default:
      return { ok: true as const };
  }
};

afterEach(() => vi.restoreAllMocks());

describe("command routing", () => {
  it("the sample set covers every UiCommand type and is contract-valid", () => {
    expect(new Set(SAMPLE_COMMANDS.map((c) => c.type))).toEqual(new Set(UI_COMMAND_TYPES));
    for (const c of SAMPLE_COMMANDS) expect(isUiCommand(c), JSON.stringify(c)).toBe(true);
  });

  it("routes every ★ command through gestureCommand and nothing else", async () => {
    const plugin = createFakePlugin();
    plugin.answer = answerFor as never;
    const engine = createAppEngine(plugin, { warn: () => {} });
    for (const cmd of SAMPLE_COMMANDS) {
      const result = await engine.command(cmd);
      expect(result.ok, cmd.type).toBe(true);
    }
    expect(plugin.sent.filter(requiresGesture)).toEqual([]);
    expect(plugin.gestured.every(requiresGesture)).toBe(true);
    expect(plugin.gestured.map((c) => c.type)).toEqual(["goLive", "resumeMic", "audioCheck", "audioCheck", "labStart", "labStart"]);
    expect(plugin.sent).toHaveLength(SAMPLE_COMMANDS.length - 6);
  });

  it("calls the plugin synchronously (inside the user gesture)", () => {
    const plugin = createFakePlugin();
    plugin.answer = answerFor as never;
    const engine = createAppEngine(plugin, { warn: () => {} });
    void engine.goLive();
    expect(plugin.calls.at(-1)?.method).toBe("gestureCommand");
    void engine.command({ type: "labStart", ...labParams("full") });
    expect(plugin.calls.at(-1)?.method).toBe("gestureCommand");
    void engine.startEngine({ groupId: "0123456789abcdef0123456789abcdef" });
    expect(plugin.calls.at(-1)?.method).toBe("startEngine");
  });

  it("maps plugin rejections with an ErrorCode to { ok: false }", async () => {
    const plugin = createFakePlugin();
    plugin.gestureCommand = () => Promise.reject({ code: "notForeground", message: "notForeground" });
    const engine = createAppEngine(plugin, { warn: () => {} });
    expect(await engine.goLive()).toEqual({ ok: false, code: "notForeground" });
  });

  it("refuses an invalid command without calling the plugin", async () => {
    const plugin = createFakePlugin();
    const engine = createAppEngine(plugin, { warn: () => {} });
    const bad = { type: "setTalkMode", mode: "shout" } as unknown as UiCommand;
    expect(await engine.command(bad)).toEqual({ ok: false, code: "invalidCommand" });
    expect(plugin.calls).toEqual([]);
  });
});

describe("startEngine", () => {
  it("starts, then sends setPrompts with every template in the rider's language", async () => {
    const plugin = createFakePlugin();
    const statuses: string[] = [];
    const engine = createAppEngine(plugin, {
      promptSettings: () => ({ language: "es", enabled: false }),
      setStatus: (s) => statuses.push(s),
      warn: () => {},
    });
    await engine.startEngine({ demo: true });
    expect(plugin.calls.map((c) => c.method)).toEqual(["startEngine", "send"]);
    expect(plugin.calls[0]?.args).toEqual({ demo: true });
    const setPrompts = plugin.sent[0];
    expect(setPrompts?.type).toBe("setPrompts");
    if (setPrompts?.type !== "setPrompts") return;
    expect(setPrompts.lang).toBe("es");
    expect(setPrompts.enabled).toBe(false);
    expect(Object.keys(setPrompts.templates).sort()).toEqual([...PROMPT_KEYS].sort());
    expect(setPrompts.templates["demo.line1"]).toBe(promptTemplates("es")["demo.line1"]);
    expect(statuses).toEqual(["starting", "running"]);
  });

  it("sends setPrompts on EVERY startEngine", async () => {
    const plugin = createFakePlugin();
    const engine = createAppEngine(plugin, { promptSettings: () => ({ language: "en", enabled: true }), warn: () => {} });
    await engine.startEngine({ groupId: "0123456789abcdef0123456789abcdef" });
    await engine.startEngine({ lab: { code: "1234" } });
    expect(plugin.sent.filter((c) => c.type === "setPrompts")).toHaveLength(2);
  });

  it("keeps running when setPrompts is refused, and reports it", async () => {
    const plugin = createFakePlugin();
    plugin.answer = () => ({ ok: false, code: "internal" });
    const warn = vi.fn();
    const engine = createAppEngine(plugin, { warn });
    await expect(engine.startEngine({ demo: true })).resolves.toBeUndefined();
    expect(warn).toHaveBeenCalledWith("setPrompts refused: internal");
  });

  it("rejects and returns to idle when the plugin cannot start", async () => {
    const plugin = createFakePlugin();
    plugin.failStart = { code: "bluetoothOff" };
    const statuses: string[] = [];
    const engine = createAppEngine(plugin, { setStatus: (s) => statuses.push(s), warn: () => {} });
    await expect(engine.startEngine({ demo: true })).rejects.toEqual({ code: "bluetoothOff" });
    expect(statuses).toEqual(["starting", "idle"]);
    expect(plugin.sent).toEqual([]);
  });
});

describe("plugin pass-throughs", () => {
  it("stamps PTT with the UI time and forwards keep-screen-on", async () => {
    const plugin = createFakePlugin();
    const engine = createAppEngine(plugin, { now: () => 1234, warn: () => {} });
    await engine.ptt(true);
    await engine.ptt(false, 99);
    await engine.setKeepScreenOn(true);
    await engine.shareFile("lab/run.jsonl", "application/jsonl");
    await engine.requestPermissions(["microphone"]);
    await engine.requestPermissions();
    expect(plugin.calls.map((c) => [c.method, c.args])).toEqual([
      ["ptt", { down: true, tUiMs: 1234 }],
      ["ptt", { down: false, tUiMs: 99 }],
      ["setKeepScreenOn", { on: true }],
      ["shareFile", { path: "lab/run.jsonl", mime: "application/jsonl" }],
      ["requestPermissions", { permissions: ["microphone"] }],
      ["requestPermissions", undefined],
    ]);
  });

  it("getInfo stores the info for the API-version check", async () => {
    const plugin = createFakePlugin();
    const setInfo = vi.fn();
    const engine = createAppEngine(plugin, { setInfo, warn: () => {} });
    const info = await engine.getInfo();
    expect(setInfo).toHaveBeenCalledWith(info);
  });

  it("drops events that break the contract and reports them", async () => {
    const plugin = createFakePlugin();
    const warn = vi.fn();
    const engine = createAppEngine(plugin, { warn });
    const seen: string[] = [];
    await engine.onEvent((e) => seen.push(e.type));
    plugin.emit({ type: "talkDenied", reason: "cap" });
    plugin.emit('{"type":"talkDenied","reason":"nope"}');
    await flush();
    expect(seen).toEqual(["talkDenied"]);
    expect(warn).toHaveBeenCalledTimes(1);
  });
});
