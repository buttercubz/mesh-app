/**
 * Framework-free press/release state machine behind BigPttButton (DESIGN §11.4 Ride).
 *
 * Invariants:
 * - Every `onDown` is matched by exactly one `onUp`; `onUp` never fires without a
 *   preceding `onDown`. A stuck open mic is the failure this guards against.
 * - Only one pointer drives the button at a time; extra fingers are ignored.
 * - `onDown` runs synchronously inside the pointer/key handler that called
 *   `press()`, so the app can issue the gesture-bound `gestureCommand` from it.
 *
 * Modes:
 * - `hold`: talk while pressed. Releases on pointerup, pointercancel, lost pointer
 *   capture, key up, page hidden, unmount, disable and mode change.
 * - `latch`: a completed tap toggles talking on, the next tap toggles it off. A press
 *   that is cancelled before it completes (pointercancel / lost capture without
 *   pointerup) undoes its own latch; page hidden, unmount, disable and mode change
 *   always release.
 */

export type PttMode = "hold" | "latch";

export type PttReleaseReason =
  | "pointerup"
  | "pointercancel"
  | "lostpointercapture"
  | "keyup"
  | "toggle"
  | "hidden"
  | "unmount"
  | "disabled"
  | "mode";

export interface PttSink {
  readonly onDown: () => void;
  readonly onUp: (reason: PttReleaseReason) => void;
}

/** Snapshot of the gesture, mirrored into React state for rendering. */
export interface PttGestureState {
  /** A pointer (or key) is currently down on the button. */
  readonly pressed: boolean;
  /** `onDown` fired and its `onUp` has not fired yet. */
  readonly talking: boolean;
}

/** Pseudo pointer id used for the keyboard (Space / Enter). */
export const KEYBOARD_POINTER_ID = -1;

export interface PttGesture {
  /** Pointer or key down. Returns the new state. */
  press(pointerId: number, mode: PttMode, sink: PttSink): PttGestureState;
  /** Pointer or key up (`reason` is `pointerup` or `keyup`). */
  release(pointerId: number, mode: PttMode, sink: PttSink, reason: "pointerup" | "keyup"): PttGestureState;
  /** The OS took the pointer away (`pointercancel` or `lostpointercapture`). */
  cancel(pointerId: number, mode: PttMode, sink: PttSink, reason: "pointercancel" | "lostpointercapture"): PttGestureState;
  /** Unconditional release: page hidden, unmount, disabled, mode change. */
  forceRelease(sink: PttSink, reason: PttReleaseReason): PttGestureState;
  /** Current state; the same object until something changes (useSyncExternalStore). */
  state(): PttGestureState;
  /** Notified after every state change. Returns the unsubscribe function. */
  subscribe(listener: () => void): () => void;
}

export function createPttGesture(): PttGesture {
  let activePointer: number | null = null;
  let talking = false;
  /** Latch mode: the active press turned talking on and has not completed yet. */
  let latchPending = false;

  let current: PttGestureState = { pressed: false, talking: false };
  const listeners = new Set<() => void>();

  /** Publishes the new state (only when it changed) and returns it. */
  const snapshot = (): PttGestureState => {
    const pressed = activePointer !== null;
    if (current.pressed !== pressed || current.talking !== talking) {
      current = { pressed, talking };
      for (const listener of [...listeners]) listener();
    }
    return current;
  };

  const stopTalking = (sink: PttSink, reason: PttReleaseReason) => {
    if (!talking) return;
    talking = false;
    sink.onUp(reason);
  };

  return {
    press(pointerId, mode, sink) {
      if (activePointer !== null) return snapshot(); // a second finger is ignored
      activePointer = pointerId;
      if (mode === "hold") {
        if (!talking) {
          talking = true;
          sink.onDown();
        }
      } else if (talking) {
        latchPending = false;
        stopTalking(sink, "toggle");
      } else {
        talking = true;
        latchPending = true;
        sink.onDown();
      }
      return snapshot();
    },

    release(pointerId, mode, sink, reason) {
      if (activePointer !== pointerId) return snapshot();
      activePointer = null;
      if (mode === "hold") {
        stopTalking(sink, reason);
      } else {
        latchPending = false; // the tap completed: the latch holds
      }
      return snapshot();
    },

    cancel(pointerId, mode, sink, reason) {
      if (activePointer !== pointerId) return snapshot();
      activePointer = null;
      if (mode === "hold" || latchPending) {
        latchPending = false;
        stopTalking(sink, reason);
      }
      return snapshot();
    },

    forceRelease(sink, reason) {
      activePointer = null;
      latchPending = false;
      stopTalking(sink, reason);
      return snapshot();
    },

    state: () => current,

    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

/** Clamps an input level to [0, 1]; NaN and non-finite values read as 0. */
export function clampLevel(level: number): number {
  if (!Number.isFinite(level) || level <= 0) return 0;
  return level >= 1 ? 1 : level;
}
