// `audioCheck` of the fake engine (DESIGN §3.7, §11.4): emits `audioCheckSample` at
// 20 Hz with a synthetic speech-like input level, so the onboarding headset check, the
// Echo test and the Settings VOX meter can be built in a browser. Phases follow the
// engine: tone → "tone"; echo → "record" then "echo" (3 s each, repeating);
// voxMeter → "meter".
import type { AudioCheckMode, AudioCheckPhase, UiEvent } from "@meshapp/engine-api";

import type { Rng } from "./rng.js";

/** `audioCheckSample` period (20 Hz). */
export const AUDIO_CHECK_SAMPLE_MS = 50;
/** Echo test record / playback phase length. */
export const ECHO_PHASE_MS = 3000;
/** Hardware input rate reported by the web stand-in. */
const INPUT_RATE = 48_000;

export class AudioCheckRunner {
  readonly #rng: Rng;
  #mode: AudioCheckMode | null = null;
  #startMs = 0;
  #lastSampleMs = -Infinity;
  #level = 0;

  constructor(rng: Rng) {
    this.#rng = rng;
  }

  get active(): boolean {
    return this.#mode !== null;
  }

  /** The check records from the microphone (echo test record phase, VOX meter). */
  get capturing(): boolean {
    return this.#mode === "echo" || this.#mode === "voxMeter";
  }

  /** Latest synthetic input level (0..1). */
  get level(): number {
    return this.#level;
  }

  start(mode: AudioCheckMode, now: number): void {
    this.#mode = mode;
    this.#startMs = now;
    this.#lastSampleMs = -Infinity;
    this.#level = 0;
  }

  stop(): void {
    this.#mode = null;
    this.#level = 0;
  }

  phaseAt(now: number): AudioCheckPhase | null {
    switch (this.#mode) {
      case null:
        return null;
      case "tone":
        return "tone";
      case "voxMeter":
        return "meter";
      case "echo":
        return Math.floor((now - this.#startMs) / ECHO_PHASE_MS) % 2 === 0 ? "record" : "echo";
    }
  }

  /** The next sample when one is due, else `null`. */
  tick(now: number): Extract<UiEvent, { type: "audioCheckSample" }> | null {
    const phase = this.phaseAt(now);
    if (phase === null || now - this.#lastSampleMs < AUDIO_CHECK_SAMPLE_MS) return null;
    this.#lastSampleMs = now;
    const listening = phase === "record" || phase === "meter";
    this.#level = listening ? this.#speechLevel((now - this.#startMs) / 1000) : 0;
    const vadP = Math.min(1, Math.max(0, (this.#level - 0.12) / 0.45));
    return {
      type: "audioCheckSample",
      level: round2(this.#level),
      vadP: round2(vadP),
      gate: vadP >= 0.5 ? "open" : "closed",
      route: "builtinSpeaker",
      inputRate: INPUT_RATE,
      phase,
    };
  }

  /** Syllable-rate envelope (≈ 4 Hz) in 1.2 s bursts every 2 s, over a noise floor. */
  #speechLevel(t: number): number {
    const burst = t % 2 < 1.2 ? 1 : 0;
    const syllable = Math.abs(Math.sin(2 * Math.PI * 2 * t));
    const floor = 0.04 + this.#rng.range(0, 0.04);
    return Math.min(1, floor + burst * syllable * this.#rng.range(0.45, 0.75));
  }
}

const round2 = (v: number): number => Math.round(v * 100) / 100;
