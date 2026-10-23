// Deterministic PRNG for the fake Lab peer (mulberry32 over an FNV-1a seed): the same
// Lab code and run order produce the same numbers, so tests and screenshots are stable.

/** FNV-1a 32-bit hash of `text`. */
export function hashSeed(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

export class Rng {
  #state: number;

  constructor(seed: number) {
    this.#state = seed >>> 0;
  }

  /** Restarts the sequence from `seed`. */
  reseed(seed: number): void {
    this.#state = seed >>> 0;
  }

  /** Uniform in [0, 1). */
  next(): number {
    this.#state = (this.#state + 0x6d2b79f5) >>> 0;
    let t = this.#state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }

  /** Uniform in [lo, hi). */
  range(lo: number, hi: number): number {
    return lo + (hi - lo) * this.next();
  }

  /** Integer in [lo, hi] (inclusive). */
  int(lo: number, hi: number): number {
    return lo + Math.floor(this.next() * (hi - lo + 1));
  }

  /** `true` with probability `p`. */
  chance(p: number): boolean {
    return this.next() < p;
  }

  /** `n` lowercase hex digits. */
  hex(n: number): string {
    let out = "";
    for (let i = 0; i < n; i++) out += this.int(0, 15).toString(16);
    return out;
  }
}
