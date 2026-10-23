import { describe, expect, it, vi } from "vitest";
import { KEYBOARD_POINTER_ID, clampLevel, createPttGesture, type PttReleaseReason } from "../src/components/ptt-gesture.js";

function sink() {
  const events: string[] = [];
  return {
    events,
    onDown: vi.fn(() => events.push("down")),
    onUp: vi.fn((reason: PttReleaseReason) => events.push(`up:${reason}`)),
  };
}

describe("ptt gesture: hold", () => {
  it("talks while pressed and releases on pointerup", () => {
    const g = createPttGesture();
    const s = sink();
    expect(g.press(1, "hold", s)).toEqual({ pressed: true, talking: true });
    expect(g.release(1, "hold", s, "pointerup")).toEqual({ pressed: false, talking: false });
    expect(s.events).toEqual(["down", "up:pointerup"]);
  });

  it("releases on pointercancel and on lost capture", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(7, "hold", s);
    g.cancel(7, "hold", s, "pointercancel");
    g.press(8, "hold", s);
    g.cancel(8, "hold", s, "lostpointercapture");
    expect(s.events).toEqual(["down", "up:pointercancel", "down", "up:lostpointercapture"]);
  });

  it("ignores a second finger and events of other pointers", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(1, "hold", s);
    g.press(2, "hold", s);
    g.release(2, "hold", s, "pointerup");
    g.cancel(3, "hold", s, "pointercancel");
    expect(g.state()).toEqual({ pressed: true, talking: true });
    g.release(1, "hold", s, "pointerup");
    expect(s.events).toEqual(["down", "up:pointerup"]);
  });

  it("never fires onUp twice (lost capture after pointerup is ignored)", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(1, "hold", s);
    g.release(1, "hold", s, "pointerup");
    g.cancel(1, "hold", s, "lostpointercapture");
    g.forceRelease(s, "hidden");
    expect(s.onUp).toHaveBeenCalledTimes(1);
  });

  it("forceRelease ends the talk and the press", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(KEYBOARD_POINTER_ID, "hold", s);
    expect(g.forceRelease(s, "hidden")).toEqual({ pressed: false, talking: false });
    // The stale keyup afterwards does nothing.
    g.release(KEYBOARD_POINTER_ID, "hold", s, "keyup");
    expect(s.events).toEqual(["down", "up:hidden"]);
  });
});

describe("ptt gesture: latch", () => {
  it("first tap latches, second tap releases", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(1, "latch", s);
    expect(g.release(1, "latch", s, "pointerup")).toEqual({ pressed: false, talking: true });
    g.press(2, "latch", s);
    expect(g.state()).toEqual({ pressed: true, talking: false });
    g.release(2, "latch", s, "pointerup");
    expect(s.events).toEqual(["down", "up:toggle"]);
  });

  it("a cancelled latching press undoes its own latch", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(1, "latch", s);
    g.cancel(1, "latch", s, "pointercancel");
    expect(g.state().talking).toBe(false);
    expect(s.events).toEqual(["down", "up:pointercancel"]);
  });

  it("a cancel after the latch completed keeps talking until hidden", () => {
    const g = createPttGesture();
    const s = sink();
    g.press(1, "latch", s);
    g.release(1, "latch", s, "pointerup");
    g.cancel(1, "latch", s, "lostpointercapture"); // stale, ignored
    expect(g.state().talking).toBe(true);
    g.forceRelease(s, "hidden");
    expect(s.events).toEqual(["down", "up:hidden"]);
  });
});

describe("ptt gesture: store", () => {
  it("notifies subscribers only on change and keeps snapshots stable", () => {
    const g = createPttGesture();
    const s = sink();
    const listener = vi.fn();
    const unsubscribe = g.subscribe(listener);
    const before = g.state();
    g.release(1, "hold", s, "pointerup"); // no-op
    expect(g.state()).toBe(before);
    expect(listener).not.toHaveBeenCalled();
    g.press(1, "hold", s);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    g.release(1, "hold", s, "pointerup");
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe("clampLevel", () => {
  it("clamps to [0, 1] and maps non-finite to 0", () => {
    expect(clampLevel(-1)).toBe(0);
    expect(clampLevel(0.4)).toBe(0.4);
    expect(clampLevel(3)).toBe(1);
    expect(clampLevel(Number.NaN)).toBe(0);
    expect(clampLevel(Number.POSITIVE_INFINITY)).toBe(0);
  });
});
