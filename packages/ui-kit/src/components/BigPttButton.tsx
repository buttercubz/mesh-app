import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import type { KeyboardEvent, PointerEvent, ReactNode } from "react";
import { cx } from "../cx.js";
import {
  KEYBOARD_POINTER_ID,
  clampLevel,
  createPttGesture,
  type PttMode,
  type PttReleaseReason,
  type PttSink,
} from "./ptt-gesture.js";

/** Engine-driven look of the button; defaults to `active` while locally talking. */
export type PttStatus = "idle" | "active" | "blocked" | "muted";

export interface BigPttButtonProps {
  /** `hold`: talk while pressed. `latch`: tap to start, tap again to stop. */
  readonly mode: PttMode;
  /** Called synchronously inside the pointerdown/keydown handler (gesture-bound). */
  readonly onDown: () => void;
  /** Exactly once per `onDown`: pointerup, pointercancel, page hidden, unmount... */
  readonly onUp: (reason: PttReleaseReason) => void;
  /** Input level 0..1, drawn as the ring around the button. */
  readonly level: number;
  /** Visible label (already translated by the app). */
  readonly label: ReactNode;
  /** Accessible name when `label` is not plain text. */
  readonly ariaLabel?: string;
  readonly status?: PttStatus;
  readonly disabled?: boolean;
  readonly className?: string;
  readonly id?: string;
}

/** Ring geometry in the 100×100 viewBox. */
const RING_STROKE = 8;
const RING_RADIUS = 50 - RING_STROKE / 2;
export const PTT_RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

/**
 * The 160 px push-to-talk / mute button of the ride screen (DESIGN §11.4).
 * Uses pointer capture so a finger sliding off the button keeps talking until it
 * lifts; releases on pointercancel and when the page is hidden so the mic never
 * sticks open.
 */
export function BigPttButton(props: BigPttButtonProps) {
  const { mode, level, label, ariaLabel, status, disabled = false, className, id } = props;
  const [gesture] = useState(createPttGesture);
  const view = useSyncExternalStore(gesture.subscribe, gesture.state, gesture.state);

  // Latest callbacks and mode for listeners attached once (visibility, unmount).
  const latest = useRef<{ sink: PttSink; mode: PttMode }>({
    sink: { onDown: props.onDown, onUp: props.onUp },
    mode,
  });
  useLayoutEffect(() => {
    latest.current = { sink: { onDown: props.onDown, onUp: props.onUp }, mode };
  });

  // Page hidden (app backgrounded, screen locked, tab switched) releases the mic.
  useEffect(() => {
    const onHidden = () => {
      if (document.visibilityState === "hidden") gesture.forceRelease(latest.current.sink, "hidden");
    };
    const onPageHide = () => gesture.forceRelease(latest.current.sink, "hidden");
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pagehide", onPageHide);
    return () => {
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pagehide", onPageHide);
      gesture.forceRelease(latest.current.sink, "unmount");
    };
  }, [gesture]);

  // Disabling the button or switching hold/latch ends any talk in progress.
  useEffect(() => {
    if (disabled) gesture.forceRelease(latest.current.sink, "disabled");
  }, [disabled, gesture]);
  const previousMode = useRef(mode);
  useEffect(() => {
    if (previousMode.current === mode) return;
    previousMode.current = mode;
    gesture.forceRelease(latest.current.sink, "mode");
  }, [mode, gesture]);

  const sink: PttSink = { onDown: props.onDown, onUp: props.onUp };

  const onPointerDown = (e: PointerEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    e.preventDefault(); // no focus-steal scroll, no text selection
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // Capture can fail for an already-released pointer or in engines without
      // pointer capture; pointerup/pointercancel on the button still release.
    }
    gesture.press(e.pointerId, mode, sink);
  };
  const onPointerUp = (e: PointerEvent<HTMLButtonElement>) => {
    gesture.release(e.pointerId, mode, sink, "pointerup");
  };
  const onPointerCancel = (e: PointerEvent<HTMLButtonElement>) => {
    gesture.cancel(e.pointerId, mode, sink, "pointercancel");
  };
  const onLostPointerCapture = (e: PointerEvent<HTMLButtonElement>) => {
    gesture.cancel(e.pointerId, mode, sink, "lostpointercapture");
  };
  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key !== " " && e.key !== "Enter") return;
    e.preventDefault(); // suppress the synthetic click
    if (disabled || e.repeat) return;
    gesture.press(KEYBOARD_POINTER_ID, mode, sink);
  };
  const onKeyUp = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key !== " " && e.key !== "Enter") return;
    e.preventDefault();
    gesture.release(KEYBOARD_POINTER_ID, mode, sink, "keyup");
  };
  const onBlur = () => {
    // Focus left while a key was held: the keyup will never reach the button.
    gesture.cancel(KEYBOARD_POINTER_ID, mode, sink, "pointercancel");
  };

  const shownStatus: PttStatus = status ?? (view.talking ? "active" : "idle");
  const dashOffset = PTT_RING_CIRCUMFERENCE * (1 - clampLevel(level));

  return (
    <button
      type="button"
      id={id}
      className={cx("mk-ptt", className)}
      disabled={disabled}
      aria-label={ariaLabel}
      aria-pressed={mode === "latch" ? view.talking : undefined}
      data-mode={mode}
      data-status={shownStatus}
      data-pressed={view.pressed ? "true" : "false"}
      data-talking={view.talking ? "true" : "false"}
      onPointerDown={onPointerDown}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onLostPointerCapture={onLostPointerCapture}
      onKeyDown={onKeyDown}
      onKeyUp={onKeyUp}
      onBlur={onBlur}
      onContextMenu={(e) => e.preventDefault()}
    >
      <svg className="mk-ptt-ring" viewBox="0 0 100 100" aria-hidden="true" focusable="false">
        <circle className="mk-ptt-ring-track" cx="50" cy="50" r={RING_RADIUS} strokeWidth={RING_STROKE} />
        <circle
          className="mk-ptt-ring-level"
          data-testid="ptt-level"
          cx="50"
          cy="50"
          r={RING_RADIUS}
          strokeWidth={RING_STROKE}
          strokeDasharray={PTT_RING_CIRCUMFERENCE}
          strokeDashoffset={dashOffset}
        />
      </svg>
      <span className="mk-ptt-label">{label}</span>
    </button>
  );
}
