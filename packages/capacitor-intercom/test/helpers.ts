// Test doubles: a controllable browser environment and an event recorder.
import type { PermissionState } from "@capacitor/core";
import type { CommandResult, LabAudio, LabParams, UiEvent } from "@meshapp/engine-api";
import { parseCommandResult, parseUiEvent } from "@meshapp/engine-api";

import type { IntercomPermission } from "../src/definitions.js";
import type { SharedFile, Visibility, WakeLockHandle, WebEnvironment } from "../src/web.js";
import { FakeLabEngine, MeshIntercomWeb } from "../src/web.js";

export class FakeEnvironment implements WebEnvironment {
  current: Visibility = "visible";
  readonly listeners = new Set<(v: Visibility) => void>();
  readonly permissions = new Map<IntercomPermission, PermissionState>();
  readonly requested: IntercomPermission[] = [];
  readonly shared: SharedFile[] = [];
  canShare = true;
  wakeLocksRequested = 0;
  wakeLocksReleased = 0;
  wakeLockSupported = true;

  visibility(): Visibility {
    return this.current;
  }

  onVisibilityChange(listener: (v: Visibility) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Simulates the page going to the background / foreground. */
  setVisibility(v: Visibility): void {
    this.current = v;
    for (const l of this.listeners) l(v);
  }

  now(): number {
    return Date.now();
  }

  /** Page query string (`location.search` in a browser). */
  search = "";

  query(): string {
    return this.search;
  }

  appVersion(): string {
    return "1.2.3-test";
  }

  async requestWakeLock(): Promise<WakeLockHandle | null> {
    if (!this.wakeLockSupported) return null;
    this.wakeLocksRequested += 1;
    return {
      release: async () => {
        this.wakeLocksReleased += 1;
      },
    };
  }

  async queryPermission(p: IntercomPermission): Promise<PermissionState> {
    return this.permissions.get(p) ?? "prompt";
  }

  async requestPermission(p: IntercomPermission): Promise<PermissionState> {
    this.requested.push(p);
    this.permissions.set(p, "granted");
    return "granted";
  }

  async shareFile(file: SharedFile): Promise<boolean> {
    if (!this.canShare) return false;
    this.shared.push(file);
    return true;
  }
}

export interface Harness {
  plugin: MeshIntercomWeb;
  env: FakeEnvironment;
  engine: FakeLabEngine;
  events: UiEvent[];
  /** Events of one type, in order. */
  of<K extends UiEvent["type"]>(type: K): Extract<UiEvent, { type: K }>[];
  /** Sends a command object through `send` and parses the result. */
  send(command: unknown): Promise<CommandResult>;
  /** Sends a command object through `gestureCommand` and parses the result. */
  gesture(command: unknown): Promise<CommandResult>;
}

/** A web plugin over the fake Lab engine with a controllable environment. */
export async function harness(seed = "test"): Promise<Harness> {
  const env = new FakeEnvironment();
  const engine = new FakeLabEngine({ now: () => env.now(), seed, appVersion: env.appVersion() });
  const plugin = new MeshIntercomWeb({ backend: engine, environment: env });
  const events: UiEvent[] = [];
  await plugin.addListener("event", ({ json }: { json: string }) => {
    // Every emitted event must satisfy the C1 contract guards.
    events.push(parseUiEvent(json));
  });
  return {
    plugin,
    env,
    engine,
    events,
    of: <K extends UiEvent["type"]>(type: K) =>
      events.filter((e): e is Extract<UiEvent, { type: K }> => e.type === type),
    send: async (command) => parseCommandResult((await plugin.send({ command: JSON.stringify(command) })).result),
    gesture: async (command) =>
      parseCommandResult((await plugin.gestureCommand({ command: JSON.stringify(command) })).result),
  };
}

/** Minimal valid LabParams. */
export const labParams = (audio: LabAudio = "off", extra: Partial<LabParams> = {}): LabParams => ({
  audio,
  anchor: "audio",
  gps: false,
  ...extra,
});
