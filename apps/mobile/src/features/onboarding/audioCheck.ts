// Audio check shared by the onboarding headset check / Echo test and the Settings VOX
// meter (WP-22, DESIGN §1.3 F0 step 3, §11.4). `audioCheck{op:"start"}` is a ★ command:
// `start()` issues it synchronously, so it must be called straight from the tap. The
// engine streams `audioCheckSample{level, vadP, gate, route, inputRate, phase}`, which
// the shell stores in useEngineStore.audioCheck; `stop()` closes the temporary session
// and runs on unmount too.
import type { AudioCheckMode } from "@meshapp/engine-api";
import { useEffect, useRef, useState } from "react";

import type { AppEngine } from "../../engine/client";
import { useEngineStore } from "../../store/useEngineStore";
import { showError } from "../group/session";

export interface AudioCheckControl {
  readonly running: AudioCheckMode | null;
  /** ★ Starts a check; call it from the tap handler (no await before it). */
  start(mode: AudioCheckMode): void;
  stop(): void;
}

export function useAudioCheck(engine: AppEngine): AudioCheckControl {
  const [running, setRunning] = useState<AudioCheckMode | null>(null);
  const runningRef = useRef<AudioCheckMode | null>(null);

  const stop = () => {
    if (runningRef.current === null) return;
    runningRef.current = null;
    setRunning(null);
    engine.command({ type: "audioCheck", op: "stop" }).catch((e: unknown) => {
      // The temporary session is closed by the engine anyway when the check ends.
      console.warn("[audioCheck] stop failed", e);
    });
  };

  const start = (mode: AudioCheckMode) => {
    useEngineStore.getState().clearAudioCheck();
    // The ★ command leaves now, inside the tap (gestureCommand).
    const pending = engine.command({ type: "audioCheck", op: "start", mode });
    runningRef.current = mode;
    setRunning(mode);
    pending.then(
      (r) => {
        if (r.ok) return;
        if (runningRef.current === mode) {
          runningRef.current = null;
          setRunning(null);
        }
        showError(r.code);
      },
      (e: unknown) => {
        console.warn("[audioCheck] start failed", e);
        runningRef.current = null;
        setRunning(null);
        showError(undefined);
      },
    );
  };

  // Leaving the screen ends the check (the mic must never stay open).
  useEffect(
    () => () => {
      if (runningRef.current === null) return;
      runningRef.current = null;
      engine.command({ type: "audioCheck", op: "stop" }).catch((e: unknown) => {
        // Unmounting: nothing left to show; the engine times the check out itself.
        console.warn("[audioCheck] stop on unmount failed", e);
      });
    },
    [engine],
  );

  return { running, start, stop };
}
