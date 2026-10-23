// Web implementation answers every C6 method (WP-17 acceptance).
import { API_VERSION } from "@meshapp/engine-api";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { INTERCOM_PERMISSIONS } from "../src/definitions.js";
import { MeshIntercomWeb } from "../src/web.js";
import { harness } from "./helpers.js";

beforeEach(() => {
  vi.useFakeTimers({ now: Date.UTC(2026, 9, 23, 9, 12, 3) });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("info and status", () => {
  it("getInfo reports web, the fake engine and the contract versions", async () => {
    const h = await harness();
    expect(await h.plugin.getInfo()).toEqual({
      platform: "web",
      engine: "platform-only",
      apiVersion: API_VERSION,
      protocolVersion: 1,
      appVersion: "1.2.3-test",
    });
  });

  it("getStatus follows startEngine/stopEngine and page visibility", async () => {
    const h = await harness();
    expect(await h.plugin.getStatus()).toEqual({ engine: "idle", live: false, uiAttached: true });
    await h.plugin.startEngine({ lab: { code: "0042" } });
    expect(await h.plugin.getStatus()).toEqual({ engine: "running", live: false, uiAttached: true });
    h.env.setVisibility("hidden");
    expect((await h.plugin.getStatus()).uiAttached).toBe(false);
    await h.plugin.stopEngine();
    expect((await h.plugin.getStatus()).engine).toBe("idle");
  });

  it("works with the default (browser) environment outside a browser", async () => {
    const plugin = new MeshIntercomWeb();
    expect((await plugin.getInfo()).appVersion).toBe("0.0.0-web");
    // No document: the page cannot be foreground-active.
    await expect(plugin.startEngine({ lab: { code: "1234" } })).rejects.toMatchObject({ code: "notForeground" });
    await plugin.dispose();
  });
});

describe("startEngine", () => {
  it("requires a visible page (gesture required)", async () => {
    const h = await harness();
    h.env.setVisibility("hidden");
    await expect(h.plugin.startEngine({ lab: { code: "4711" } })).rejects.toMatchObject({ code: "notForeground" });
  });

  it("validates the options", async () => {
    const h = await harness();
    await expect(h.plugin.startEngine({ lab: { code: "47" } })).rejects.toMatchObject({ code: "invalidParams" });
    await expect(h.plugin.startEngine({ lab: { code: "4711" }, demo: true })).rejects.toMatchObject({
      code: "invalidParams",
    });
    await expect(h.plugin.startEngine({ demo: true })).rejects.toMatchObject({ code: "notSupported" });
    await expect(h.plugin.startEngine({ groupId: "00".repeat(16) })).rejects.toMatchObject({ code: "unknownGroup" });
  });

  it("emits a snapshot, then the scripted peer joins the Lab code", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    expect(h.of("snapshot").at(-1)?.s).toMatchObject({ engine: "running", session: null, mesh: { links: 0 } });
    await vi.advanceTimersByTimeAsync(1600);
    const joined = h.of("notice").find((n) => n.key === "labPeerJoined");
    expect(joined?.params["labNode"]).toEqual(expect.any(Number));
    expect(h.of("snapshot").at(-1)?.s.mesh).toMatchObject({ links: 1, neighbors: 1 });
  });

  it("emits snapshots at most at 10 Hz and only when changed", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    const before = h.of("snapshot").length;
    await vi.advanceTimersByTimeAsync(1000);
    expect(h.of("snapshot").length - before).toBe(0);
    await vi.advanceTimersByTimeAsync(5000);
    expect(h.of("snapshot").length - before).toBe(1); // the peer join
  });

  it("stops snapshots in the background and sends a full one on return", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    h.env.setVisibility("hidden");
    const before = h.of("snapshot").length;
    await vi.advanceTimersByTimeAsync(3000);
    expect(h.of("snapshot").length).toBe(before);
    h.env.setVisibility("visible");
    expect(h.of("snapshot").length).toBe(before + 1);
    expect(h.of("snapshot").at(-1)?.s.device.background).toBe(false);
  });
});

describe("ptt", () => {
  it("rejects before startEngine and stamps the fast path when running", async () => {
    const h = await harness();
    await expect(h.plugin.ptt({ down: true })).rejects.toMatchObject({ code: "engineNotRunning" });
    await h.plugin.startEngine({ lab: { code: "4711" } });
    await h.plugin.ptt({ down: true, tUiMs: 123.5 });
    expect(h.engine.lastPtt).toEqual({ down: true, tUiMs: 123.5, receivedAtMs: Date.now() });
    await h.plugin.ptt({ down: false });
    expect(h.engine.lastPtt).toMatchObject({ down: false, tUiMs: undefined });
  });

  it("validates its arguments", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    await expect(h.plugin.ptt({ down: "yes" as unknown as boolean })).rejects.toMatchObject({ code: "invalidParams" });
    await expect(h.plugin.ptt({ down: true, tUiMs: Number.NaN })).rejects.toMatchObject({ code: "invalidParams" });
  });
});

describe("setKeepScreenOn", () => {
  it("takes and releases a screen wake lock", async () => {
    const h = await harness();
    await h.plugin.setKeepScreenOn({ on: true });
    await h.plugin.setKeepScreenOn({ on: true });
    expect(h.env.wakeLocksRequested).toBe(1);
    expect(h.plugin.keepScreenOn).toBe(true);
    await h.plugin.setKeepScreenOn({ on: false });
    expect(h.env.wakeLocksReleased).toBe(1);
    expect(h.plugin.keepScreenOn).toBe(false);
  });

  it("is reset on every background transition", async () => {
    const h = await harness();
    await h.plugin.setKeepScreenOn({ on: true });
    h.env.setVisibility("hidden");
    await vi.advanceTimersByTimeAsync(0);
    expect(h.plugin.keepScreenOn).toBe(false);
    expect(h.env.wakeLocksReleased).toBe(1);
    h.env.setVisibility("visible");
    expect(h.plugin.keepScreenOn).toBe(false);
  });

  it("resolves without a Wake Lock API (best effort)", async () => {
    const h = await harness();
    h.env.wakeLockSupported = false;
    await h.plugin.setKeepScreenOn({ on: true });
    expect(h.plugin.keepScreenOn).toBe(true);
  });
});

describe("permissions", () => {
  it("checkPermissions reports every intercom permission", async () => {
    const h = await harness();
    h.env.permissions.set("microphone", "denied");
    const status = await h.plugin.checkPermissions();
    expect(Object.keys(status)).toEqual([...INTERCOM_PERMISSIONS]);
    expect(status.microphone).toBe("denied");
    expect(status.camera).toBe("prompt");
  });

  it("requestPermissions asks only for the requested ones, in order", async () => {
    const h = await harness();
    const status = await h.plugin.requestPermissions({ permissions: ["location", "microphone"] });
    expect(h.env.requested).toEqual(["microphone", "location"]);
    expect(status).toMatchObject({ microphone: "granted", location: "granted", camera: "prompt" });
    await h.plugin.requestPermissions();
    expect(h.env.requested.slice(2)).toEqual([...INTERCOM_PERMISSIONS]);
  });
});

describe("other methods", () => {
  it("scanInviteQr, openAppSettings, requestBatteryExemption", async () => {
    const h = await harness();
    expect(await h.plugin.scanInviteQr()).toEqual({ cancelled: true });
    await expect(h.plugin.openAppSettings()).rejects.toMatchObject({ code: "notSupported" });
    expect(await h.plugin.requestBatteryExemption()).toEqual({ granted: true });
  });

  it("setNotificationCatalog validates like the iOS bridge, then is a no-op", async () => {
    const h = await harness();
    await expect(
      h.plugin.setNotificationCatalog({ title: "convLink Voice", templates: { "notices.captureLost": "Mic lost" } }),
    ).resolves.toBeUndefined();
    const bad = h.plugin.setNotificationCatalog as (o: unknown) => Promise<void>;
    await expect(bad.call(h.plugin, { templates: {} })).rejects.toMatchObject({ code: "invalidParams" });
    await expect(bad.call(h.plugin, { title: "x", templates: ["a"] })).rejects.toMatchObject({
      code: "invalidParams",
    });
    expect(h.events).toEqual([]);
  });

  it("shareFile rejects unknown paths and missing share support", async () => {
    const h = await harness();
    await expect(h.plugin.shareFile({ path: "lab/nope.jsonl", mime: "text/plain" })).rejects.toMatchObject({
      code: "invalidParams",
    });
  });

  it("removes listeners on dispose", async () => {
    const h = await harness();
    await h.plugin.startEngine({ lab: { code: "4711" } });
    await h.plugin.dispose();
    const count = h.events.length;
    await vi.advanceTimersByTimeAsync(5000);
    expect(h.events.length).toBe(count);
    expect((await h.plugin.getStatus()).engine).toBe("idle");
  });
});

