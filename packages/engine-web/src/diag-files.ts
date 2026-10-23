// The web simulator's ridelog/1 diagnostics recorder (diagnostics spec §3.1 "Web sim"):
// the same JSONL line format as the native runtime (crates/mesh-lab/schema/
// ridelog-1.schema.json), kept in memory only — never in web storage, like
// `lab-files.ts` — and exported as one `.jsonl` per session, so `tools/lab-analyze ride`
// can be exercised end to end without phones. The WASM core does not emit diagnostics
// records yet (stage 2), so the simulated phone's lines come from what this page sees:
// engine start/stop, every command and its answer, UI events, and 1 Hz samples built
// from the local snapshot.
import type { Snapshot, UiEvent } from "@meshapp/engine-api";

export const RIDELOG_MIME = "application/x-ndjson";
/** RAM ring per session while recording is off (and the cap while on). */
export const RING_LINES = 4096;
export const MARK_LABELS = ["crackle", "cantHear", "notHeard", "lag", "dropped", "other"] as const;
export type MarkLabel = (typeof MARK_LABELS)[number];

type Scalar = string | number | boolean | null;
export type DiagFields = Record<string, Scalar | number[]>;

/** Device identity written in every `hdr`. */
export interface DiagDevice {
  model: string;
  os: string;
  app: string;
}

/** Recorder state (`diagStatus`). */
export interface DiagState {
  enabled: boolean;
  sessionId: string | null;
  rideKey: string | null;
  node: number | null;
  bytes: number;
  sessions: number;
  oldestMs: number | null;
}

interface Session {
  id: string;
  startedMs: number;
  originMs: number;
  rideKey: string | null;
  node: number | null;
  role: string | null;
  lines: string[];
  bytes: number;
  open: boolean;
}

const pad = (n: number, w = 2): string => String(n).padStart(w, "0");
const r3 = (v: number): number => Math.round(v * 1000) / 1000;

/** `d-YYYYMMDD-HHMM-xxxx` from a wall time and a random 4-character base-36 tail. */
export function sessionIdFor(wallMs: number, random: () => number = Math.random): string {
  const d = new Date(wallMs);
  const tail = Array.from({ length: 4 }, () => "0123456789abcdefghijklmnopqrstuvwxyz"[Math.floor(random() * 36) % 36]).join("");
  return `d-${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}-${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}-${tail}`;
}

const utcDate = (wallMs: number): number => {
  const d = new Date(wallMs);
  return d.getUTCFullYear() * 10_000 + (d.getUTCMonth() + 1) * 100 + d.getUTCDate();
};

async function sha256(parts: Uint8Array[]): Promise<Uint8Array> {
  const len = parts.reduce((n, p) => n + p.length, 0);
  const buf = new Uint8Array(len);
  let o = 0;
  for (const p of parts) {
    buf.set(p, o);
    o += p.length;
  }
  return new Uint8Array(await globalThis.crypto.subtle.digest("SHA-256", buf));
}

const hex = (b: Uint8Array): string => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const u32le = (v: number): Uint8Array => new Uint8Array([v & 255, (v >>> 8) & 255, (v >>> 16) & 255, (v >>> 24) & 255]);

/** `rideKey` of a group session: SHA-256("ridelog" ‖ group_id ‖ u32le UTC date)[0..4] (as mesh-lab). */
export async function groupRideKey(groupIdHex: string, wallMs: number): Promise<string | null> {
  if (!/^[0-9a-fA-F]{32}$/.test(groupIdHex)) return null;
  const id = new Uint8Array(16);
  for (let i = 0; i < 16; i++) id[i] = parseInt(groupIdHex.slice(i * 2, i * 2 + 2), 16);
  const d = await sha256([new TextEncoder().encode("ridelog"), id, u32le(utcDate(wallMs))]);
  return hex(d.subarray(0, 4));
}

/** `rideKey` of a Lab session: `lab-` + the public Lab tag (SHA-256("cvic/lab" ‖ u16le code)). */
export async function labRideKey(code: number): Promise<string> {
  const d = await sha256([new TextEncoder().encode("cvic/lab"), new Uint8Array([code & 255, (code >>> 8) & 255])]);
  const tag = d[0]! | (d[1]! << 8);
  return `lab-${tag.toString(16).padStart(4, "0")}`;
}

/** Strings that could be key material or user text are redacted (as the runtime does). */
export function redact(v: string): string {
  return v.length > 64 || /[0-9a-fA-F]{24,}/.test(v) || /[A-Za-z0-9+/=_-]{32,}/.test(v) ? "<redacted>" : v;
}

const IDENT = /^[a-z][A-Za-z0-9]*$/;

export class RideLogRecorder {
  readonly #device: DiagDevice;
  readonly #now: () => number;
  readonly #sessions: Session[] = [];
  #enabled = true;
  #lastSampleMs = -Infinity;

  constructor(device: DiagDevice, now: () => number = Date.now) {
    this.#device = device;
    this.#now = now;
  }

  get enabled(): boolean {
    return this.#enabled;
  }

  setEnabled(on: boolean): void {
    this.#enabled = on;
  }

  #current(): Session | undefined {
    const s = this.#sessions.at(-1);
    return s?.open ? s : undefined;
  }

  /** Starts a session (engine start); ends the previous one. */
  begin(mode: string, rideKey: string | null): void {
    const now = this.#now();
    this.end();
    const s: Session = {
      id: sessionIdFor(now),
      startedMs: now,
      originMs: now,
      rideKey,
      node: null,
      role: null,
      lines: [],
      bytes: 0,
      open: true,
    };
    this.#sessions.push(s);
    this.#push(s, this.#header(s));
    this.event("rt", "engineStart", { mode });
    this.event("rt", "clkAnchor", { wallMs: now, monoUs: 0 });
  }

  /** Ends the open session (`engineStop` + `end`). */
  end(): void {
    const s = this.#current();
    if (s === undefined) return;
    this.event("rt", "engineStop", {});
    this.#push(s, { t: "end", ts: this.#ts(s), reason: "sessionEnd", lines: s.lines.length + 1, suppressed: 0, redacted: 0 });
    s.open = false;
  }

  #header(s: Session): Record<string, unknown> {
    const d = new Date(s.startedMs);
    return {
      t: "hdr",
      v: 1,
      schema: "ridelog/1",
      seg: 1,
      sessionId: s.id,
      rideKey: s.rideKey,
      startedAt: d.toISOString(),
      monoUs: 0,
      device: { platform: "web", model: this.#device.model, os: this.#device.os, app: this.#device.app, build: "debug", engine: "wasm" },
      node: s.node,
      role: s.role,
      caps: { l2cap: false, aware: false },
      settings: {},
      privacy: { gps: "off", names: "none" },
      verbose: false,
    };
  }

  #ts(s: Session): number {
    return r3(Math.max(0, this.#now() - s.originMs) / 1000);
  }

  #push(s: Session, line: Record<string, unknown>): void {
    const text = JSON.stringify(line);
    s.lines.push(text);
    s.bytes += text.length + 1;
    // Recording off: only a RAM ring is kept (the header stays first).
    const cap = this.#enabled ? RING_LINES * 16 : RING_LINES;
    while (s.lines.length > cap) {
      const [dropped] = s.lines.splice(1, 1);
      s.bytes -= (dropped?.length ?? 0) + 1;
    }
  }

  /** One `e` line (ev and field keys must be camelCase ids; strings are redacted). */
  event(src: "core" | "rt" | "plat" | "ui", ev: string, fields: DiagFields): boolean {
    const s = this.#current();
    if (s === undefined || !IDENT.test(ev)) return false;
    const line: Record<string, unknown> = { t: "e", ts: this.#ts(s), src, ev };
    for (const [k, v] of Object.entries(fields)) {
      if (!IDENT.test(k) || k in line) return false;
      line[k] = typeof v === "string" ? redact(v) : Array.isArray(v) ? v.slice(0, 64) : v;
    }
    this.#push(s, line);
    return true;
  }

  /** A UI note (src `ui`, e.g. `cmd` with the command id and its answer). */
  note(ev: string, fields: DiagFields): boolean {
    return this.event("ui", ev, fields);
  }

  /** A user marker with a fixed label. */
  mark(label: string): boolean {
    const s = this.#current();
    if (s === undefined || !(MARK_LABELS as readonly string[]).includes(label)) return false;
    this.#push(s, { t: "mark", ts: this.#ts(s), src: "ui", label });
    return true;
  }

  /** The node id and role, once the local snapshot shows them. */
  identify(node: number | null, role: string | null): void {
    const s = this.#current();
    if (s === undefined || (node === s.node && role === s.role)) return;
    s.node = node;
    s.role = role;
    this.event("core", "session", { rideKey: s.rideKey, node, role });
  }

  /** Records what the page sees of an engine event. */
  uiEvent(e: UiEvent): void {
    const ev = e as unknown as Record<string, unknown>;
    const type = typeof ev["type"] === "string" ? ev["type"] : "";
    if (type === "snapshot") {
      const snap = ev["s"] as Snapshot | undefined;
      if (snap !== undefined) this.snapshot(snap);
      return;
    }
    const key = typeof ev["key"] === "string" ? ev["key"] : typeof ev["code"] === "string" ? ev["code"] : null;
    if (type === "notice") this.event("core", "notice", { key });
    else if (type === "error") this.event("core", "uiError", { code: key });
    else if (IDENT.test(type)) this.event("core", "uiEvent", { type, key });
  }

  /** 1 Hz sample from the local snapshot (the browser has no per-link counters). */
  snapshot(snap: Snapshot): void {
    if (snap.session !== null) this.identify(snap.session.myNodeId, snap.session.role);
    const now = this.#now();
    const s = this.#current();
    if (s === undefined || now - this.#lastSampleMs < 1000) return;
    this.#lastSampleMs = now;
    const m = snap.mesh;
    this.#push(s, {
      t: "s",
      ts: this.#ts(s),
      links: [],
      mesh: { nbrs: m.neighbors, reach: m.reachable, islands: m.islands },
      floor: { talkers: snap.talkers.length, me: snap.session?.talk ?? "idle" },
      audio: {
        running: snap.audio.captureOk || snap.audio.renderOk,
        route: snap.audio.route,
        ladder: snap.session?.ladder ?? null,
      },
      jb: [],
      jbTotal: null,
      dev: {
        bat: snap.device.battery,
        chg: null,
        therm: snap.device.thermal,
        app: snap.device.background ? "background" : "foreground",
        locked: null,
        call: null,
      },
      anchor: null,
      bt: snap.device.bluetooth,
      aware: "unsupported",
      distM: null,
      cpuMs: null,
      fx: { droppedDiag: 0, suppressed: 0, redacted: 0, droppedLines: 0 },
    });
  }

  status(): DiagState {
    const s = this.#current() ?? this.#sessions.at(-1);
    return {
      enabled: this.#enabled,
      sessionId: s?.id ?? null,
      rideKey: s?.rideKey ?? null,
      node: s?.node ?? null,
      bytes: this.#enabled ? this.#sessions.reduce((n, x) => n + x.bytes, 0) : 0,
      sessions: this.#enabled ? this.#sessions.length : 0,
      oldestMs: this.#enabled ? (this.#sessions[0]?.startedMs ?? null) : null,
    };
  }

  /** Sessions as files (`ridelog-<sessionId>.jsonl`), newest first; `last` = only the newest. */
  files(scope: "last" | "all"): { name: string; data: string; mime: string }[] {
    const list = [...this.#sessions].reverse();
    const chosen = scope === "last" ? list.slice(0, 1) : list;
    return chosen.map((s) => ({ name: `ridelog-${s.id}.jsonl`, data: `${s.lines.join("\n")}\n`, mime: RIDELOG_MIME }));
  }

  list(): { sessionId: string; bytes: number; segments: number; modifiedMs: number }[] {
    return [...this.#sessions].reverse().map((s) => ({ sessionId: s.id, bytes: s.bytes, segments: 1, modifiedMs: s.startedMs }));
  }

  /** Forgets every closed session; the open one restarts from its header. */
  clear(): void {
    const open = this.#current();
    this.#sessions.length = 0;
    if (open !== undefined) {
      open.lines = [];
      open.bytes = 0;
      this.#sessions.push(open);
      this.#push(open, this.#header(open));
    }
  }
}
