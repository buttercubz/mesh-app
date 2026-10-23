// The single event subscription (WP-18, DESIGN §11.3): events → stores, notices and
// errors → translated toasts, snapshot request on foreground return.
import type { ToastInput } from "@meshapp/ui-kit";
import { NOTICE_KEYS, UI_EVENT_TYPES, isUiEvent } from "@meshapp/engine-api";
import type { UiEvent } from "@meshapp/engine-api";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  NOTICE_TONES,
  createAppEngine,
  engineEventsRefCount,
  errorText,
  handleEngineEvent,
  noticeToast,
  startEngineEvents,
} from "../src/engine/client";
import { applyLanguage } from "../src/i18n/index";
import { useEngineStore } from "../src/store/useEngineStore";
import { useLabStore } from "../src/store/useLabStore";
import { labSampleFixture, labSummaryFixture, snapshotFixture } from "./fixtures";
import { createFakePlugin, flush } from "./helpers";

const EVENTS: UiEvent[] = [
  { type: "snapshot", s: snapshotFixture() },
  { type: "talkDenied", reason: "cap" },
  { type: "notice", key: "memberJoined", params: { n: 1, names: "Ana" } },
  { type: "routeChanged", route: "builtinSpeaker" },
  { type: "audioCheckSample", level: 0.5, vadP: 0.9, gate: "open", route: "hfpWideband", inputRate: 16000, phase: "meter" },
  { type: "labProgress", runId: "r1", elapsedS: 2, sample: labSampleFixture },
  { type: "labFinished", runId: "r1", summary: labSummaryFixture, files: ["lab/r1.jsonl"] },
  { type: "error", code: "bluetoothOff", params: {} },
];

function sinks() {
  const toasts: ToastInput[] = [];
  return { toasts, sinks: { toast: (t: ToastInput) => void toasts.push(t), now: () => 42 } };
}

beforeEach(() => {
  applyLanguage("en");
  useEngineStore.getState().reset();
  useLabStore.getState().reset();
});

describe("handleEngineEvent", () => {
  it("fixtures cover every UiEvent type and are contract-valid", () => {
    expect(new Set(EVENTS.map((e) => e.type))).toEqual(new Set(UI_EVENT_TYPES));
    for (const e of EVENTS) expect(isUiEvent(e), e.type).toBe(true);
  });

  it("routes every event into the stores and toasts", () => {
    const { toasts, sinks: s } = sinks();
    for (const e of EVENTS) handleEngineEvent(e, s);
    const engine = useEngineStore.getState();
    expect(engine.snapshot?.session?.groupName).toBe("Sunday ride");
    expect(engine.snapshotAt).toBe(42);
    expect(engine.lastTalkDenied).toEqual({ reason: "cap", at: 42 });
    expect(engine.route).toBe("builtinSpeaker");
    expect(engine.audioCheck).toMatchObject({ level: 0.5, phase: "meter", at: 42 });
    expect(engine.lastError).toEqual({ code: "bluetoothOff", params: {}, at: 42 });
    expect(useLabStore.getState().lastFinished?.runId).toBe("r1");
    expect(toasts.map((t) => t.title)).toEqual(["Joined: Ana", "Bluetooth is off. Turn it on to reach the other riders."]);
    expect(toasts[1]?.tone).toBe("danger");
  });

  it("translates notices in the current language with their params", () => {
    applyLanguage("es");
    expect(noticeToast("memberLost", { n: 2, names: "Ana, Luis" }).title).toBe("Fuera de alcance (2): Ana, Luis");
    expect(noticeToast("batteryLow", { percent: 15 }).title).toBe("Batería baja (15 %)");
    applyLanguage("en");
    expect(noticeToast("removed", { group: "Sunday" })).toMatchObject({ title: "You were removed from Sunday", tone: "danger", durationMs: 0 });
  });

  it("every notice has a tone and a translation; paired notices share a toast slot", () => {
    for (const key of NOTICE_KEYS) {
      expect(NOTICE_TONES[key]).toBeDefined();
      expect(noticeToast(key, { name: "x", n: 1, names: "x", islands: 2, group: "g", percent: 10, labNode: 1 }).title).not.toBe(key);
    }
    expect(noticeToast("captureLost", {}).key).toBe(noticeToast("captureRestored", {}).key);
    expect(noticeToast("bluetoothOff", {}).key).toBe(noticeToast("bluetoothOn", {}).key);
  });

  it("falls back to errors.unknown", () => {
    expect(errorText(undefined)).toBe("Something went wrong. Try again.");
  });
});

describe("startEngineEvents", () => {
  let foregroundListeners: (() => void)[] = [];
  const foreground = {
    onForeground(listener: () => void) {
      foregroundListeners.push(listener);
      return () => {
        foregroundListeners = foregroundListeners.filter((l) => l !== listener);
      };
    },
  };

  afterEach(() => {
    foregroundListeners = [];
  });

  it("keeps ONE plugin subscription however many holders", async () => {
    const plugin = createFakePlugin();
    const engine = createAppEngine(plugin, { warn: () => {} });
    const { sinks: s } = sinks();
    const release1 = startEngineEvents(engine, foreground, s);
    const release2 = startEngineEvents(engine, foreground, s);
    await flush();
    expect(engineEventsRefCount()).toBe(2);
    expect(plugin.listenerCount()).toBe(1);
    plugin.emit({ type: "talkDenied", reason: "congestion" });
    expect(useEngineStore.getState().lastTalkDenied?.reason).toBe("congestion");
    release1();
    release1();
    expect(plugin.listenerCount()).toBe(1);
    release2();
    await flush();
    expect(engineEventsRefCount()).toBe(0);
    expect(plugin.listenerCount()).toBe(0);
    expect(foregroundListeners).toHaveLength(0);
  });

  it("requests a full snapshot on foreground return only while running", async () => {
    const plugin = createFakePlugin();
    const engine = createAppEngine(plugin, { warn: () => {} });
    const release = startEngineEvents(engine, foreground, sinks().sinks);
    await flush();
    for (const l of foregroundListeners) l();
    expect(plugin.sent).toEqual([]);
    useEngineStore.getState().setStatus("running");
    for (const l of foregroundListeners) l();
    await flush();
    expect(plugin.sent).toEqual([{ type: "requestSnapshot" }]);
    release();
  });
});
