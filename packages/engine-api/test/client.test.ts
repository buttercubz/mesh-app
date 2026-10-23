// createEngineClient: routing (★ only through gestureCommand), result decoding,
// rejection mapping and the validated event stream.
import { describe, expect, it, vi } from "vitest";
import { ContractError, createEngineClient, rejectionCode } from "../src/index.js";
import type { EnginePlugin, UiCommand, UiEvent } from "../src/index.js";
import { commandFixtures, eventFixtures, resultFixtures } from "./fixtures.js";

type Listener = (event: { json: string }) => void;

/** A fake MeshIntercom plugin: answers `{ ok: true }` unless `answer` says otherwise. */
function fakePlugin(answer: (command: UiCommand) => unknown = () => ({ ok: true })) {
  const listeners = new Set<Listener>();
  const reply = (opts: { command: string }) =>
    Promise.resolve({ result: JSON.stringify(answer(JSON.parse(opts.command) as UiCommand)) });
  const plugin = {
    send: vi.fn(reply),
    gestureCommand: vi.fn(reply),
    addListener: vi.fn((_name: "event", cb: Listener) => {
      listeners.add(cb);
      return Promise.resolve({
        remove: () => {
          listeners.delete(cb);
          return Promise.resolve();
        },
      });
    }),
    emit: (json: string) => listeners.forEach((cb) => cb({ json })),
    listenerCount: () => listeners.size,
  };
  return plugin satisfies EnginePlugin;
}

/** The fixture result whose `for` matches the command type, else `{ ok: true }`. */
const answerFor = (command: UiCommand) =>
  resultFixtures.find((f) => f.for === command.type && f.result.ok)?.result ?? { ok: true };

describe("command routing", () => {
  it.each(commandFixtures.map((f) => [f.name, f] as const))(
    "%s goes through the right plugin method",
    async (_name, fixture) => {
      const plugin = fakePlugin(answerFor);
      const client = createEngineClient(plugin);
      const command = fixture.command as unknown as UiCommand;
      const result = await client.command(command);
      expect(result.ok).toBe(true);
      const gestureCalls = plugin.gestureCommand.mock.calls.length;
      const sendCalls = plugin.send.mock.calls.length;
      expect([gestureCalls, sendCalls]).toEqual(fixture.requiresGesture ? [1, 0] : [0, 1]);
      const used = fixture.requiresGesture ? plugin.gestureCommand : plugin.send;
      expect(JSON.parse(used.mock.calls[0]![0].command)).toEqual(fixture.command);
    },
  );

  it("calls the plugin synchronously so ★ commands stay inside the user gesture", () => {
    const plugin = fakePlugin();
    const client = createEngineClient(plugin);
    void client.command({ type: "goLive" });
    expect(plugin.gestureCommand).toHaveBeenCalledTimes(1);
    void client.command({ type: "audioCheck", op: "start", mode: "tone" });
    expect(plugin.gestureCommand).toHaveBeenCalledTimes(2);
    expect(plugin.send).not.toHaveBeenCalled();
  });

  it("refuses an invalid command without calling the plugin", async () => {
    const plugin = fakePlugin();
    const client = createEngineClient(plugin);
    const bad = { type: "labStart", bench: "T0", params: { anchor: "audio", gps: false } };
    await expect(client.command(bad as unknown as UiCommand)).resolves.toEqual({
      ok: false,
      code: "invalidCommand",
    });
    expect(plugin.send).not.toHaveBeenCalled();
    expect(plugin.gestureCommand).not.toHaveBeenCalled();
  });
});

describe("command results", () => {
  it("returns typed payloads", async () => {
    const client = createEngineClient(fakePlugin(answerFor));
    const res = await client.command({ type: "getInvite", groupId: "5f2c" });
    if (!res.ok) throw new Error("expected ok");
    expect(res.hideAfterMs).toBe(60000);
    expect(res.qrPng.startsWith("data:image/png;base64,")).toBe(true);
    const runs = await client.command({ type: "labListRuns" });
    expect(runs.ok && runs.runs.length).toBeGreaterThan(0);
  });

  it("passes refusals through", async () => {
    const client = createEngineClient(fakePlugin(() => ({ ok: false, code: "noSession" })));
    await expect(client.command({ type: "lockGroup" })).resolves.toEqual({
      ok: false,
      code: "noSession",
    });
  });

  it("maps a plugin rejection that carries an ErrorCode to a refusal", async () => {
    const plugin = fakePlugin();
    plugin.gestureCommand.mockImplementationOnce(() =>
      Promise.reject(Object.assign(new Error("notForeground"), { code: "notForeground" })),
    );
    const client = createEngineClient(plugin);
    await expect(client.command({ type: "resumeMic" })).resolves.toEqual({
      ok: false,
      code: "notForeground",
    });
  });

  it("rethrows any other plugin failure", async () => {
    const plugin = fakePlugin();
    const boom = new Error("bridge exploded");
    plugin.send.mockImplementationOnce(() => Promise.reject(boom));
    await expect(createEngineClient(plugin).command({ type: "labStop" })).rejects.toBe(boom);
  });

  it("rejects results that break the contract or have the wrong kind", async () => {
    const garbage = createEngineClient(fakePlugin(() => ({ ok: true, message: "done!" })));
    await expect(garbage.command({ type: "labStop" })).rejects.toThrow(ContractError);
    const wrongKind = createEngineClient(fakePlugin(() => ({ ok: true })));
    await expect(wrongKind.command({ type: "listGroups" })).rejects.toThrow(/expected groups/);
  });

  it("finds error codes in rejections", () => {
    expect(rejectionCode({ code: "useGestureCommand" })).toBe("useGestureCommand");
    expect(rejectionCode({ code: "UNIMPLEMENTED", message: "notForeground" })).toBe("notForeground");
    expect(rejectionCode("bluetoothOff")).toBe("bluetoothOff");
    expect(rejectionCode(new Error("nope"))).toBeUndefined();
    expect(rejectionCode(null)).toBeUndefined();
  });
});

describe("events", () => {
  it("delivers every valid event and drops invalid ones", async () => {
    const plugin = fakePlugin();
    const onContractError = vi.fn();
    const client = createEngineClient(plugin, { onContractError });
    const seen: UiEvent[] = [];
    const unsubscribe = await client.onEvent((e) => seen.push(e));
    for (const f of eventFixtures) plugin.emit(JSON.stringify(f.event));
    plugin.emit(JSON.stringify({ type: "notice", key: "alone", message: "You are alone" }));
    plugin.emit("{oops");
    expect(seen).toEqual(eventFixtures.map((f) => f.event));
    expect(onContractError).toHaveBeenCalledTimes(2);
    expect(onContractError.mock.calls[0]![0]).toBeInstanceOf(ContractError);
    await unsubscribe();
    expect(plugin.listenerCount()).toBe(0);
  });

  it("filters by type", async () => {
    const plugin = fakePlugin();
    const client = createEngineClient(plugin);
    const notices: string[] = [];
    await client.on("notice", (e) => notices.push(e.key));
    for (const f of eventFixtures) plugin.emit(JSON.stringify(f.event));
    const expected = eventFixtures.filter((f) => f.event.type === "notice").map((f) => f.event.key);
    expect(notices).toEqual(expected);
    expect(notices.length).toBeGreaterThan(0);
  });
});
