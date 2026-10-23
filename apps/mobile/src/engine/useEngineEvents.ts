// The app's single engine event subscription (WP-18, DESIGN §11.3): events → stores,
// notices/errors → i18n → toasts. Snapshots are the source of truth; WebView timers are
// not trusted while backgrounded, so a full snapshot is requested on every foreground
// return (convLink lesson, §11.1).
import type { UiEvent } from "@meshapp/engine-api";
import type { ToastInput } from "@meshapp/ui-kit";
import { toaster } from "@meshapp/ui-kit";
import { useEffect } from "react";

import { useEngineStore } from "../store/useEngineStore";
import { useLabStore } from "../store/useLabStore";
import type { AppEngine } from "./appEngine";
import { engine as appEngine } from "./appEngine";
import { errorToast, noticeToast } from "./notices";

export interface EventSinks {
  toast(input: ToastInput): void;
  now(): number;
}

const defaultSinks: EventSinks = {
  toast: (input) => {
    toaster.show(input);
  },
  now: () => Date.now(),
};

/** Routes one validated UiEvent into the stores (pure apart from the sinks). */
export function handleEngineEvent(event: UiEvent, sinks: EventSinks = defaultSinks): void {
  const engineStore = useEngineStore.getState();
  switch (event.type) {
    case "snapshot":
      engineStore.applySnapshot(event.s, sinks.now());
      return;
    case "talkDenied":
      // Shown by the ride screen's busy banner (from the store) and the busy tone.
      engineStore.talkDenied(event.reason, sinks.now());
      return;
    case "notice":
      sinks.toast(noticeToast(event.key, event.params));
      return;
    case "routeChanged":
      engineStore.routeChanged(event.route);
      return;
    case "audioCheckSample": {
      const { type: _type, ...sample } = event;
      engineStore.audioCheckSample(sample, sinks.now());
      return;
    }
    case "labProgress":
      useLabStore.getState().progress(event.runId, event.elapsedS, event.sample);
      return;
    case "labFinished":
      useLabStore.getState().finished(event.runId, event.summary, event.files);
      return;
    case "error":
      engineStore.error(event.code, { ...event.params }, sinks.now());
      sinks.toast(errorToast(event.code, event.params));
      return;
  }
}

export interface ForegroundSource {
  /** Calls `listener` whenever the app returns to the foreground; returns unsubscribe. */
  onForeground(listener: () => void): () => void;
}

/** Page visibility as the foreground signal (WKWebView and Android WebView fire it). */
export const documentForeground: ForegroundSource = {
  onForeground(listener) {
    if (typeof document === "undefined") return () => {};
    const onChange = () => {
      if (document.visibilityState === "visible") listener();
    };
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  },
};

let refCount = 0;
let teardown: (() => void) | null = null;

/**
 * Starts the single subscription (idempotent, reference-counted) and returns its
 * release function. Non-React code and tests use this; components use the hook.
 */
export function startEngineEvents(
  engine: AppEngine = appEngine,
  foreground: ForegroundSource = documentForeground,
  sinks: EventSinks = defaultSinks,
): () => void {
  refCount += 1;
  if (refCount === 1) {
    let unsubscribe: (() => Promise<void>) | null = null;
    let cancelled = false;
    engine
      .onEvent((event) => handleEngineEvent(event, sinks))
      .then(
        (off) => {
          if (cancelled) void off();
          else unsubscribe = off;
        },
        (e: unknown) => {
          // No event stream means no snapshots; surface it instead of failing silently.
          console.error("[engine] event subscription failed", e);
        },
      );
    const offForeground = foreground.onForeground(() => {
      if (useEngineStore.getState().status !== "running") return;
      engine.command({ type: "requestSnapshot" }).catch((e: unknown) => {
        // The next periodic snapshot will catch up; a failed request is only logged.
        console.warn("[engine] requestSnapshot on foreground failed", e);
      });
    });
    teardown = () => {
      cancelled = true;
      offForeground();
      if (unsubscribe !== null) void unsubscribe();
    };
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    refCount -= 1;
    if (refCount === 0 && teardown !== null) {
      teardown();
      teardown = null;
    }
  };
}

/** Number of live holders of the subscription (tests). */
export function engineEventsRefCount(): number {
  return refCount;
}

/** React hook: keeps the single engine subscription alive while mounted (App root). */
export function useEngineEvents(): void {
  useEffect(() => startEngineEvents(), []);
}
