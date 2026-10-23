// Optional WebAudio playback for the browser simulator (DESIGN §11.2): while somebody
// else talks, speech-like noise plays (band-limited noise with a syllable-rate
// envelope), one voice per talker, scaled by the talker's level. The simulated frames
// are SimCodec frames, not audio, so this is a cue that the floor logic works, not a
// rendering of real speech. No audio assets are shipped.
import type { Snapshot, Talker } from "@meshapp/engine-api";

/** The part of AudioContext the player uses (a fake in tests). */
export interface AudioContextLike {
  readonly currentTime: number;
  readonly sampleRate: number;
  readonly state: string;
  readonly destination: AudioNode;
  resume(): Promise<void>;
  close(): Promise<void>;
  createBuffer(channels: number, length: number, sampleRate: number): AudioBuffer;
  createBufferSource(): AudioBufferSourceNode;
  createBiquadFilter(): BiquadFilterNode;
  createGain(): GainNode;
}

/** Talkers that produce sound: everybody but this phone. */
export function audibleTalkers(s: Snapshot): Talker[] {
  const me = s.session?.myNodeId;
  return s.talkers.filter((t) => t.nodeId !== me);
}

interface Voice {
  source: AudioBufferSourceNode;
  gain: GainNode;
}

/** Peak gain of one voice (several talkers mix below clipping). */
const VOICE_GAIN = 0.18;
/** Seconds of noise in the looped buffer. */
const LOOP_S = 2;

/** Plays speech-like noise for each remote talker of the latest snapshot. */
export class TalkerNoisePlayer {
  readonly #ctx: AudioContextLike;
  readonly #voices = new Map<number, Voice>();
  #buffer: AudioBuffer | undefined;
  #closed = false;

  constructor(ctx: AudioContextLike) {
    this.#ctx = ctx;
  }

  /** Creates a player on a new browser AudioContext; `undefined` without WebAudio. */
  static create(): TalkerNoisePlayer | undefined {
    const Ctor = (globalThis as { AudioContext?: new () => AudioContextLike }).AudioContext;
    if (Ctor === undefined) return undefined;
    return new TalkerNoisePlayer(new Ctor());
  }

  /** Browsers start contexts suspended until a user gesture: call from a tap. */
  async resume(): Promise<void> {
    if (this.#ctx.state === "suspended") await this.#ctx.resume();
  }

  /** Remote talkers currently sounding. */
  get playing(): number[] {
    return [...this.#voices.keys()].sort((a, b) => a - b);
  }

  /** Follows a snapshot: starts voices for new talkers, stops gone ones. */
  update(s: Snapshot): void {
    if (this.#closed) return;
    const talkers = audibleTalkers(s);
    const now = new Set(talkers.map((t) => t.nodeId));
    for (const [id, v] of this.#voices) {
      if (!now.has(id)) this.#stopVoice(id, v);
    }
    for (const t of talkers) {
      const level = Math.min(1, Math.max(0.2, t.level));
      const existing = this.#voices.get(t.nodeId);
      if (existing === undefined) {
        this.#startVoice(t.nodeId, level);
      } else {
        existing.gain.gain.setTargetAtTime(VOICE_GAIN * level, this.#ctx.currentTime, 0.05);
      }
    }
  }

  /** Silences every voice. */
  silence(): void {
    for (const [id, v] of this.#voices) this.#stopVoice(id, v);
  }

  /** Stops playback and closes the context. */
  async close(): Promise<void> {
    this.silence();
    this.#closed = true;
    await this.#ctx.close();
  }

  #noise(): AudioBuffer {
    if (this.#buffer !== undefined) return this.#buffer;
    const rate = this.#ctx.sampleRate;
    const n = Math.floor(rate * LOOP_S);
    const buf = this.#ctx.createBuffer(1, n, rate);
    const data = buf.getChannelData(0);
    // Deterministic noise (xorshift) under a ~4 Hz syllable envelope with pauses.
    let x = 0x9e3779b9;
    for (let i = 0; i < n; i++) {
      x ^= x << 13;
      x ^= x >>> 17;
      x ^= x << 5;
      const white = ((x >>> 0) / 0xffffffff) * 2 - 1;
      const t = i / rate;
      const syllable = Math.max(0, Math.sin(2 * Math.PI * 4 * t));
      const phrase = t % 1 < 0.8 ? 1 : 0.1;
      data[i] = white * syllable * phrase;
    }
    this.#buffer = buf;
    return buf;
  }

  #startVoice(id: number, level: number): void {
    const ctx = this.#ctx;
    const source = ctx.createBufferSource();
    source.buffer = this.#noise();
    source.loop = true;
    // Voices start at different points of the loop so two talkers do not sound alike.
    const offset = (id % 7) * (LOOP_S / 7);
    const band = ctx.createBiquadFilter();
    band.type = "bandpass";
    band.frequency.value = 600 + (id % 5) * 150;
    band.Q.value = 0.9;
    const gain = ctx.createGain();
    gain.gain.value = 0;
    gain.gain.setTargetAtTime(VOICE_GAIN * level, ctx.currentTime, 0.03);
    source.connect(band);
    band.connect(gain);
    gain.connect(ctx.destination);
    source.start(ctx.currentTime, offset);
    this.#voices.set(id, { source, gain });
  }

  #stopVoice(id: number, v: Voice): void {
    const t = this.#ctx.currentTime;
    v.gain.gain.setTargetAtTime(0, t, 0.03);
    v.source.stop(t + 0.2);
    this.#voices.delete(id);
  }
}
