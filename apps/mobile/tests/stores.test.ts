// Stores (WP-18, DESIGN §11.3, §13.1): persisted settings/groups with name/version/
// migrate/merge and no secrets; engine store API compatibility; bounded Lab ring.
import { DISPLAY_NAME_MAX_BYTES } from "@meshapp/engine-api";
import { beforeEach, describe, expect, it } from "vitest";

import { APP_API_VERSION, apiCompat, useEngineStore } from "../src/store/useEngineStore";
import {
  GROUPS_STORE_NAME,
  GROUP_META_FIELDS,
  sanitizeGroupsData,
  useGroupsStore,
} from "../src/store/useGroupsStore";
import { LAB_SAMPLE_RING, pushBounded, useLabStore } from "../src/store/useLabStore";
import {
  SETTINGS_STORE_NAME,
  SETTINGS_STORE_VERSION,
  clampDisplayName,
  defaultSettings,
  migrateSettings,
  sanitizeSettings,
  useSettingsStore,
  vehicleDefaults,
} from "../src/store/useSettingsStore";
import { appStorage } from "../src/store/storage";
import { GROUP_ID, labSampleFixture, labSummaryFixture, snapshotFixture } from "./fixtures";

const utf8Length = (s: string) => new TextEncoder().encode(s).length;

describe("settings store", () => {
  beforeEach(() => useSettingsStore.getState().reset());

  it("defaults: dark theme, motorcycle VOX, prompts on, distance off", () => {
    const s = defaultSettings("es");
    expect(s).toMatchObject({ language: "es", theme: "dark", vehicle: "moto", defaultMode: "vox", spokenPrompts: true, shareDistance: false });
  });

  it("applies the §1.4 vehicle defaults", () => {
    useSettingsStore.getState().setVehicle("car");
    expect(useSettingsStore.getState()).toMatchObject({ vehicle: "car", defaultMode: "ptt", builtinMicTx: true, keepScreenOnMounted: true });
    expect(vehicleDefaults("moto")).toMatchObject({ defaultMode: "vox", builtinMicTx: false, keepScreenOnMounted: false });
  });

  it("sanitises untrusted persisted values", () => {
    const s = sanitizeSettings({ language: "fr", theme: "neon", vehicle: "truck", haptics: "yes", secret: "x", displayName: 4 }, defaultSettings("en"));
    expect(s).toEqual(defaultSettings("en"));
    expect(Object.keys(s)).not.toContain("secret");
  });

  it("migrates v0 field names", () => {
    const s = migrateSettings({ lang: "es", promptsEnabled: false, vehicle: "car" }, 0);
    expect(s).toMatchObject({ language: "es", spokenPrompts: false, vehicle: "car" });
  });

  it("persists under its name and version, settings only", () => {
    useSettingsStore.getState().update({ language: "es", haptics: true });
    const raw = appStorage.getItem(SETTINGS_STORE_NAME);
    expect(typeof raw).toBe("string");
    const parsed = JSON.parse(raw as string) as { state: Record<string, unknown>; version: number };
    expect(parsed.version).toBe(SETTINGS_STORE_VERSION);
    expect(parsed.state["language"]).toBe("es");
    expect(Object.keys(parsed.state).sort()).toEqual(Object.keys(defaultSettings()).sort());
  });

  it("merge rehydrates through the sanitiser", async () => {
    appStorage.setItem(
      SETTINGS_STORE_NAME,
      JSON.stringify({ state: { language: "es", theme: "light", inviteUrl: "convlinkvoice://join#x" }, version: 1 }),
    );
    await useSettingsStore.persist.rehydrate();
    const state = useSettingsStore.getState() as unknown as Record<string, unknown>;
    expect(state["language"]).toBe("es");
    expect(state["theme"]).toBe("light");
    expect(state["inviteUrl"]).toBeUndefined();
    expect(appStorage.getItem(SETTINGS_STORE_NAME)).not.toContain("inviteUrl");
  });

  it("clamps display names to 24 UTF-8 bytes at grapheme boundaries", () => {
    expect(clampDisplayName("  Ana   María ")).toBe("Ana María");
    const long = "👨‍👩‍👧‍👦".repeat(3) + "abcdef";
    const clamped = clampDisplayName(long);
    expect(utf8Length(clamped)).toBeLessThanOrEqual(DISPLAY_NAME_MAX_BYTES);
    expect(clamped).toBe("");
    const accents = clampDisplayName("ñandú".repeat(6));
    expect(utf8Length(accents)).toBeLessThanOrEqual(24);
    expect(accents.normalize()).toBe(accents);
    expect(clampDisplayName("a".repeat(30))).toBe("a".repeat(24));
  });
});

describe("groups store (metadata only)", () => {
  beforeEach(() => useGroupsStore.setState({ groups: [], activeGroupId: null }));

  it("keeps only id/name/role/lastUsed, even from a tampered storage entry", async () => {
    appStorage.setItem(
      GROUPS_STORE_NAME,
      JSON.stringify({
        state: {
          groups: [{ id: GROUP_ID, name: "Sunday", role: "leader", lastUsed: 5, epochSecret: "deadbeef", invite: "convlinkvoice://join#x" }],
          activeGroupId: GROUP_ID,
          psk: "secret",
        },
        version: 1,
      }),
    );
    await useGroupsStore.persist.rehydrate();
    const state = useGroupsStore.getState();
    expect(state.groups).toEqual([{ id: GROUP_ID, name: "Sunday", role: "leader", lastUsed: 5 }]);
    expect((state as unknown as Record<string, unknown>)["psk"]).toBeUndefined();
    expect(state.activeGroupId).toBe(GROUP_ID);
    const persisted = JSON.parse(appStorage.getItem(GROUPS_STORE_NAME) as string) as { state: { groups: object[] } };
    for (const g of persisted.state.groups) expect(Object.keys(g).sort()).toEqual([...GROUP_META_FIELDS].sort());
    expect(appStorage.getItem(GROUPS_STORE_NAME)).not.toMatch(/secret|invite|psk/i);
  });

  it("upserts, touches, syncs from the engine and removes", () => {
    const s = useGroupsStore.getState();
    s.upsert({ id: GROUP_ID, name: "Sunday", role: "member", lastUsed: 1 });
    s.upsert({ id: "f".repeat(32), name: "Night", role: "leader", lastUsed: 2 });
    expect(useGroupsStore.getState().lastUsed()?.name).toBe("Night");
    useGroupsStore.getState().touch(GROUP_ID, 10);
    expect(useGroupsStore.getState().lastUsed()?.id).toBe(GROUP_ID);
    useGroupsStore.getState().setActive(GROUP_ID);
    useGroupsStore.getState().syncFromEngine(
      [
        { groupId: GROUP_ID, name: "Sunday ride", role: "leader", vehicle: "moto", members: 3, epoch: 1, closed: false },
        { groupId: "e".repeat(32), name: "Closed", role: "member", vehicle: "car", members: 2, epoch: 0, closed: true },
      ],
      50,
    );
    expect(useGroupsStore.getState().groups).toEqual([{ id: GROUP_ID, name: "Sunday ride", role: "leader", lastUsed: 10 }]);
    useGroupsStore.getState().remove(GROUP_ID);
    expect(useGroupsStore.getState()).toMatchObject({ groups: [], activeGroupId: null });
  });

  it("rejects malformed ids and unknown active groups", () => {
    expect(sanitizeGroupsData({ groups: [{ id: "nothex", name: "x" }], activeGroupId: "nothex" })).toEqual({ groups: [], activeGroupId: null });
    useGroupsStore.getState().setActive(GROUP_ID);
    expect(useGroupsStore.getState().activeGroupId).toBeNull();
  });
});

describe("engine store", () => {
  beforeEach(() => useEngineStore.getState().reset());

  it("accepts the same API major, refuses another", () => {
    expect(apiCompat(APP_API_VERSION)).toBe("ok");
    expect(apiCompat("1.9")).toBe("ok");
    expect(apiCompat("2.0")).toBe("mismatch");
    expect(apiCompat("x")).toBe("mismatch");
    expect(apiCompat(undefined)).toBe("unknown");
  });

  it("derives status and route from snapshots", () => {
    const store = useEngineStore.getState();
    store.applySnapshot(snapshotFixture(), 100);
    expect(useEngineStore.getState()).toMatchObject({ status: "running", route: "hfpWideband", snapshotAt: 100 });
    store.applySnapshot(snapshotFixture({ engine: "crashed" }));
    expect(useEngineStore.getState().status).toBe("idle");
  });
});

describe("lab store", () => {
  beforeEach(() => useLabStore.getState().reset());

  it("bounds the live sample ring at 600", () => {
    for (let i = 0; i < LAB_SAMPLE_RING + 50; i += 1) useLabStore.getState().progress("r1", i / 2, labSampleFixture);
    const { samples, activeRunId } = useLabStore.getState();
    expect(activeRunId).toBe("r1");
    expect(samples).toHaveLength(LAB_SAMPLE_RING);
    expect(samples[0]?.elapsedS).toBe(25);
    expect(pushBounded([1, 2, 3], 4, 3)).toEqual([2, 3, 4]);
  });

  it("a new run restarts the ring; finishing records the summary", () => {
    useLabStore.getState().setRuns([
      { runId: "r1", bench: "T0", audio: "off", anchor: "audio", startedAt: "2026-10-22T10:00:00Z", durationS: null, verdict: null, files: [] },
    ]);
    useLabStore.getState().progress("r0", 1, labSampleFixture);
    useLabStore.getState().progress("r1", 1, labSampleFixture);
    expect(useLabStore.getState().samples).toHaveLength(1);
    useLabStore.getState().finished("r1", labSummaryFixture, ["lab/r1.jsonl"]);
    const s = useLabStore.getState();
    expect(s.activeRunId).toBeNull();
    expect(s.runs[0]).toMatchObject({ verdict: "go", durationS: 60, files: ["lab/r1.jsonl"] });
    expect(s.lastFinished?.runId).toBe("r1");
  });
});
