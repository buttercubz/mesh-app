// Ride model (WP-22): reach counts, banners, the VOX/loudspeaker rule and the vehicle
// defaults (car → PTT + keep screen on), against the mesh-api snapshot fixtures.
import type { Snapshot } from "@meshapp/engine-api";
import { describe, expect, it } from "vitest";

import { vehicleDefaults } from "../../store/useSettingsStore";
import { fixtureSnapshot } from "../group/testing";
import {
  BUSY_BANNER_MS,
  batteryPercent,
  compassKey,
  formatDistance,
  heardBy,
  keepRideScreenOn,
  qualityBars,
  rideBanners,
  startMode,
  voiceReach,
  voxAllowed,
} from "./model";

const kinds = (s: Snapshot | null, extra: { at?: number | null; reason?: "cap" | "preempted" | "congestion" } = {}) =>
  rideBanners({ snapshot: s, lastTalkDeniedAt: extra.at ?? null, lastTalkDeniedReason: extra.reason ?? null, now: 10_000 }).map(
    (b) => b.kind,
  );

describe("reach", () => {
  const ride = fixtureSnapshot("snapshot ride");

  it("counts who hears me and whose voice reaches, best effort apart", () => {
    expect(heardBy(ride)).toEqual({ k: 1, n: ride.members.length - 1 });
    const reach = voiceReach(ride);
    expect(reach.m).toBe(ride.members.length - 1);
    expect(reach.bestEffort).toBe(1);
    expect(reach.n).toBeGreaterThanOrEqual(reach.bestEffort);
  });

  it("is empty without a snapshot", () => {
    expect(heardBy(null)).toEqual({ k: 0, n: 0 });
    expect(voiceReach(null)).toEqual({ n: 0, m: 0, bestEffort: 0 });
  });
});

describe("vehicle defaults and the loudspeaker rule (§1.4)", () => {
  it("car defaults to PTT and keep-screen-on; motorcycle to VOX", () => {
    expect(vehicleDefaults("car")).toMatchObject({ defaultMode: "ptt", keepScreenOnMounted: true });
    expect(vehicleDefaults("moto")).toMatchObject({ defaultMode: "vox", keepScreenOnMounted: false });
  });

  it("never offers VOX on the phone loudspeaker", () => {
    expect(voxAllowed("builtinSpeaker")).toBe(false);
    expect(voxAllowed("carBluetooth")).toBe(true);
    expect(voxAllowed("hfpWideband")).toBe(true);
    expect(startMode("vox", "builtinSpeaker")).toBe("ptt");
    expect(startMode("vox", "hfpNarrowband")).toBe("vox");
    expect(startMode("muted", "builtinSpeaker")).toBe("muted");
  });

  it("keeps the ride screen awake in PTT, and in a car while mounted", () => {
    expect(keepRideScreenOn("ptt", "moto", false)).toBe(true);
    expect(keepRideScreenOn("vox", "moto", false)).toBe(false);
    expect(keepRideScreenOn("vox", "car", true)).toBe(true);
    expect(keepRideScreenOn("vox", "car", false)).toBe(false);
  });
});

describe("rideBanners", () => {
  it("shows Bluetooth off, the ladder with Resume, not live, wind mute and islands for the degraded lobby", () => {
    const degraded = fixtureSnapshot("snapshot lobby degraded");
    const banners = rideBanners({ snapshot: degraded, lastTalkDeniedAt: null, now: 0 });
    expect(banners.map((b) => b.kind)).toEqual(["bluetoothOff", "ladderL2", "notLive", "island"]);
    expect(banners.find((b) => b.kind === "ladderL2")?.action).toBe("resumeMic");
    expect(banners.find((b) => b.kind === "notLive")?.action).toBe("goLive");
    expect(banners.find((b) => b.kind === "island")?.params).toEqual({ n: 2 });
  });

  it("asks for Bluetooth in Settings when it is not allowed", () => {
    const s = fixtureSnapshot("snapshot ride");
    const denied = { ...s, device: { ...s.device, bluetooth: "unauthorized" as const } };
    const b = rideBanners({ snapshot: denied, lastTalkDeniedAt: null, now: 0 });
    expect(b[0]).toMatchObject({ kind: "bluetoothDenied", action: "openSettings" });
  });

  it("says alone when no other rider is connected, and mic paused when capture died", () => {
    const s = fixtureSnapshot("snapshot ride");
    const alone: Snapshot = {
      ...s,
      members: s.members.map((m) => (m.status === "me" ? m : { ...m, status: "lost" as const })),
      audio: { ...s.audio, captureOk: false },
    };
    expect(kinds(alone)).toEqual(["micPaused", "alone"]);
  });

  it("never shows mic paused for the wind-muted phone mic, and offers Use phone mic", () => {
    const s = fixtureSnapshot("snapshot ride");
    const wind: Snapshot = { ...s, audio: { ...s.audio, captureOk: false, txMutedNoHeadset: true } };
    const banners = rideBanners({ snapshot: wind, lastTalkDeniedAt: null, now: 0 });
    expect(banners.map((b) => b.kind)).not.toContain("micPaused");
    expect(banners.find((b) => b.kind === "txMutedNoHeadset")?.action).toBe("usePhoneMic");
  });

  it("keeps the busy banner for a few seconds after talkDenied, with the reason", () => {
    const s = fixtureSnapshot("snapshot ride");
    expect(kinds(s, { at: 10_000 - 100 })).toContain("busy");
    expect(kinds(s, { at: 10_000 - BUSY_BANNER_MS - 1 })).not.toContain("busy");
    expect(kinds(s, { at: 9_900, reason: "preempted" })).toContain("preempted");
    expect(kinds(s, { at: 9_900, reason: "congestion" })).toContain("congestion");
    const blocked = { ...s, session: s.session === null ? null : { ...s.session, talk: "blocked" as const } };
    const busy = rideBanners({ snapshot: blocked, lastTalkDeniedAt: null, now: 0 }).find((b) => b.kind === "busy");
    expect(busy?.params).toEqual({ n: 2 });
  });

  it("is quiet for a healthy ride and without a snapshot", () => {
    expect(kinds(fixtureSnapshot("snapshot ride"))).toEqual([]);
    expect(kinds(null)).toEqual([]);
  });
});

describe("formatting", () => {
  it("rounds distances, batteries, bars and bearings for a glance", () => {
    expect(formatDistance(142, "en-US")).toBe("140 m");
    expect(formatDistance(1530, "es")).toBe("1,5 km");
    expect(batteryPercent(0.555)).toBe(56);
    expect(batteryPercent(null)).toBeNull();
    expect([0, 20, 48, 72, 100].map(qualityBars)).toEqual([0, 1, 2, 3, 4]);
    expect(compassKey(181)).toBe("compass.s");
    expect(compassKey(-45)).toBe("compass.nw");
    expect(compassKey(359)).toBe("compass.n");
  });
});
